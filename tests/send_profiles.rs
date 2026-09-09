use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, binding_profile_identity, compose_binding_profiles,
    generate_c, generate_runtime_c_named, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = "version 1; message State @id(2) { required packed fixed32 data[100] @id(1); } message Request @id(3) {} message Response @id(4) { required uint32 value @id(1); }";
const RPC: &str = "profile version 1; rpc Query { request = Request; response = Response; }";
const SEND: &str = "profile version 1; send State { delivery = unreliable; }";
const RECEIVE: &str = "profile version 1; latest State { delivery = unreliable; }";

#[test]
fn composed_profiles_are_order_independent_and_reject_cross_file_conflicts() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let resolve =
        |s: &str| analyze_binding_profile(&parse_binding_profile(s).unwrap(), &schema).unwrap();
    let rpc = resolve(RPC);
    let send = resolve(SEND);
    let combined = compose_binding_profiles(&[rpc.clone(), send.clone()]).unwrap();
    let reversed = compose_binding_profiles(&[send.clone(), rpc.clone()]).unwrap();
    assert_eq!(combined, reversed);
    let flat = resolve(&format!("{RPC} send State {{ delivery = unreliable; }}"));
    assert_eq!(combined, flat);
    assert_eq!(
        binding_profile_identity(&combined),
        binding_profile_identity(&flat)
    );
    assert_ne!(
        binding_profile_identity(&combined),
        binding_profile_identity(&rpc)
    );
    assert!(
        compose_binding_profiles(&[rpc.clone(), rpc.clone()])
            .unwrap_err()
            .0
            .contains("duplicate RPC")
    );
    assert!(
        compose_binding_profiles(&[send.clone(), send])
            .unwrap_err()
            .0
            .contains("duplicate send")
    );
    let receive = resolve(RECEIVE);
    assert!(compose_binding_profiles(&[receive.clone(), receive]).is_err());
    let plain_rpc = resolve("profile version 1; send Request { delivery = reliable; }");
    assert!(
        compose_binding_profiles(&[rpc, plain_rpc])
            .unwrap_err()
            .0
            .contains("also has a plain")
    );
    for invalid in [
        "send Missing { delivery = unreliable; }",
        "send State { delivery = maybe; }",
        "send State { delivery = unreliable; } send State { delivery = reliable; }",
        "rpc Query { request = Request; response = Response; } send Response { delivery = reliable; }",
    ] {
        assert!(
            analyze_binding_profile(
                &parse_binding_profile(&format!("profile version 1; {invalid}")).unwrap(),
                &schema
            )
            .is_err()
        );
    }
}

#[test]
fn send_only_bounds_and_duplex_delivery_are_explicit() {
    for body in [
        "required bytes data @id(1);",
        "required bytes<4096> data @id(1);",
    ] {
        let schema = analyze_schema(
            &parse_schema(&format!("version 1; message State @id(2) {{ {body} }}")).unwrap(),
        )
        .unwrap();
        let profile =
            analyze_binding_profile(&parse_binding_profile(SEND).unwrap(), &schema).unwrap();
        let generated = generate_runtime_c_named(&schema, &profile, "demo", "sender").unwrap();
        assert!(
            generated
                .header
                .contains("#define SENDER_HAS_DEFAULT_ENDPOINT 0")
        );
    }
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(&format!("{RECEIVE} send State {{ delivery = reliable; }}"))
            .unwrap(),
        &schema,
    )
    .unwrap();
    let generated = generate_runtime_c_named(&schema, &profile, "demo", "duplex").unwrap();
    assert_eq!(
        generated
            .header
            .matches("duplex_endpoint_send_state(")
            .count(),
        1
    );
    assert!(
        generated
            .header
            .contains("message, WL_DELIVERY_RELIABLE, now_ms")
    );
    assert!(generated.source.contains("WL_EVT_UNRELIABLE_RX"));
}

#[test]
fn outbound_bound_without_a_mailbox_runs_against_real_core() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let codec = generate_c(&schema, "demo").unwrap();
    let temp = tempdir().unwrap();
    for (name, text) in [
        ("demo.h", codec.header),
        ("demo.c", codec.source),
        ("demo_values.h", codec.values_header),
        ("demo_bindings.h", codec.bindings_header),
        ("demo_bindings.c", codec.bindings_source),
    ] {
        fs::write(temp.path().join(name), text).unwrap();
    }
    for (name, source) in [
        ("sender", SEND),
        ("receiver", RECEIVE),
        (
            "reliable_sender",
            "profile version 1; send State { delivery = reliable; }",
        ),
        (
            "reliable_receiver",
            "profile version 1; latest State { delivery = reliable; }",
        ),
    ] {
        let profile =
            analyze_binding_profile(&parse_binding_profile(source).unwrap(), &schema).unwrap();
        let generated = generate_runtime_c_named(&schema, &profile, "demo", name).unwrap();
        if name == "sender" {
            assert!(!generated.header.contains("state_latest"));
            assert!(!generated.header.contains("endpoint_read_state"));
            assert!(generated.header.contains("endpoint_send_state"));
        }
        for (suffix, text) in [
            ("runtime.h", generated.header),
            ("runtime.c", generated.source),
            ("endpoint.h", generated.endpoint_header),
        ] {
            fs::write(temp.path().join(format!("{name}_{suffix}")), text).unwrap();
        }
    }
    fs::write(
        temp.path().join("test.c"),
        include_str!("fixtures/send_profiles.c"),
    )
    .unwrap();
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let mut cc = Command::new("cc");
    cc.args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(root.join("tests/support"))
        .arg("-I")
        .arg(temp.path());
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let mut sources = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect::<Vec<_>>();
    sources.sort();
    cc.args(sources)
        .arg(root.join("adapters/loopback/src/loopback.c"));
    for file in [
        "demo.c",
        "demo_bindings.c",
        "sender_runtime.c",
        "receiver_runtime.c",
        "reliable_sender_runtime.c",
        "reliable_receiver_runtime.c",
        "test.c",
    ] {
        cc.arg(temp.path().join(file));
    }
    let binary = temp.path().join("send-test");
    let output = cc.arg("-o").arg(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let header_test = temp.path().join("headers.cpp");
    fs::write(
        &header_test,
        "#include \"sender_endpoint.h\"\n#include \"reliable_sender_endpoint.h\"\n",
    )
    .unwrap();
    let output = Command::new("c++")
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg(header_test)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
