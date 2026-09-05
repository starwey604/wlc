use std::{fs, path::PathBuf, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c,
    generate_runtime_c_named, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = r#"
version 1;
message State = 2 { required uint32 sequence = 1; }
message Alarm = 3 { required uint32 code = 1; }
message Request = 4 { optional uint32 operation_id = 1; required int32 input = 2; }
message Response = 5 { optional uint32 operation_id = 1; optional int32 status = 2; required int32 output = 3; }
message Unrelated = 6 { optional bytes content = 1; }
"#;
const PROFILE: &str = r#"
profile version 1;
latest State { delivery = unreliable; }
fifo Alarm { delivery = reliable; }
rpc Execute {
  request = Request; response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable; response_delivery = reliable;
}
"#;

#[test]
fn default_endpoints_compile_and_run_with_real_core() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    let generated = generate_runtime_c(&schema, &profile, "demo").unwrap();
    assert!(
        generated
            .header
            .contains("#define DEMO_HAS_DEFAULT_ENDPOINT 1")
    );
    assert_eq!(
        generated,
        generate_runtime_c(&schema, &profile, "demo").unwrap()
    );
    let codec = generate_c(&schema, "demo").unwrap();
    let alternate = generate_runtime_c_named(&schema, &profile, "demo", "peer").unwrap();
    let temp = tempdir().unwrap();
    for (name, contents) in [
        ("demo.h", codec.header), ("demo.c", codec.source),
        ("demo_bindings.h", codec.bindings_header), ("demo_bindings.c", codec.bindings_source),
        ("demo_runtime.h", generated.header), ("demo_runtime.c", generated.source),
        ("peer_runtime.h", alternate.header), ("peer_runtime.c", alternate.source),
        ("test.c", include_str!("fixtures/default_endpoint.c").to_owned()),
        ("headers.cpp", "#include \"demo_runtime.h\"\n#include \"peer_runtime.h\"\nstatic demo_endpoint_t a;\nstatic peer_endpoint_t b;\nint main() { return demo_endpoint_init(&a, 1) + peer_endpoint_init(&b, 2); }\n".to_owned()),
    ] { fs::write(temp.path().join(name), contents).unwrap(); }
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned()
        });
    let mut command = Command::new("cc");
    command
        .args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(temp.path());
    let mut sources = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect::<Vec<_>>();
    sources.sort();
    command
        .args(sources)
        .arg(root.join("adapters/loopback/src/loopback.c"));
    for name in [
        "demo.c",
        "demo_bindings.c",
        "demo_runtime.c",
        "peer_runtime.c",
        "test.c",
    ] {
        command.arg(temp.path().join(name));
    }
    let executable = temp.path().join("endpoint-test");
    let result = command.arg("-o").arg(&executable).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(executable).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new("c++")
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
        .arg("-I")
        .arg(temp.path())
        .arg(temp.path().join("headers.cpp"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn unbounded_or_oversized_selected_messages_require_custom_storage() {
    for body in [
        "optional bytes content = 2;",
        "optional bytes<4096> content = 2;",
    ] {
        let source = SCHEMA.replace("required int32 input = 2;", body);
        let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let profile =
            analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
        let generated = generate_runtime_c(&schema, &profile, "demo").unwrap();
        assert!(
            generated
                .header
                .contains("#define DEMO_HAS_DEFAULT_ENDPOINT 0")
        );
        assert!(!generated.header.contains("} demo_endpoint_t;"));
    }
}

#[test]
fn endpoint_type_names_are_reserved() {
    let schema = analyze_schema(
        &parse_schema("version 1; message DemoEndpoint = 1 { required uint32 value = 1; }")
            .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile("profile version 1; latest DemoEndpoint { delivery = unreliable; }")
            .unwrap(),
        &schema,
    )
    .unwrap();
    assert!(
        generate_runtime_c(&schema, &profile, "demo")
            .unwrap_err()
            .0
            .contains("demo_endpoint_t")
    );
}

#[test]
fn endpoint_rpc_and_retained_names_cannot_collide() {
    let schema = analyze_schema(&parse_schema(&SCHEMA.replace("State", "Start")).unwrap()).unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(&PROFILE.replace("State", "Start").replace("Execute", "Send"))
            .unwrap(),
        &schema,
    )
    .unwrap();
    assert!(
        generate_runtime_c(&schema, &profile, "demo")
            .unwrap_err()
            .0
            .contains("demo_endpoint_send_start")
    );
}
