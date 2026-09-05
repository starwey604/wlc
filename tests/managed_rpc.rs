use std::{fs, path::PathBuf, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, binding_profile_identity, generate_c,
    generate_runtime_c, generate_runtime_c_named, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = r#"
version 1;
message Request @id(2) { required int32 input @id(1); }
message Response @id(3) { required int32 output @id(1); }
message AuxiliaryRequest @id(4) {}
message AuxiliaryResponse @id(5) {}
"#;
const PROFILE: &str = r#"
profile version 1;
rpc Execute {
  request = Request; response = Response;
  request_delivery = reliable; response_delivery = reliable;
}
rpc Auxiliary {
  request = AuxiliaryRequest; response = AuxiliaryResponse;
  request_delivery = unreliable; response_delivery = unreliable;
}
"#;

#[test]
fn managed_rpc_profiles_have_no_business_metadata_or_partial_mapping() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    assert!(
        profile
            .rpc_services
            .iter()
            .all(|service| service.is_managed())
    );
    let codec = generate_c(&schema, "demo").unwrap();
    assert!(!codec.header.contains("operation_id"));
    assert!(!codec.header.contains("application_status"));
    let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
    assert!(runtime.header.contains("demo_execute_call_t"));
    assert!(!runtime.header.contains("rpc_encode_scratch"));
    assert!(
        runtime
            .header
            .contains("#define DEMO_ENDPOINT_MAX_PAYLOAD 18U")
    );
    assert!(
        runtime
            .source
            .contains("config->rpc_client_response_capacity = 18U")
    );
    assert!(
        runtime
            .source
            .contains("config->execute_canonical_request_capacity = 6U")
    );
    assert_eq!(
        runtime,
        generate_runtime_c(&schema, &profile, "demo").unwrap()
    );
    for mapping in [
        "request_operation_id",
        "response_operation_id",
        "response_status",
    ] {
        let partial = PROFILE.replacen(
            "request = Request;",
            &format!("request = Request; {mapping} = input;"),
            1,
        );
        let error = analyze_binding_profile(&parse_binding_profile(&partial).unwrap(), &schema)
            .unwrap_err();
        assert!(error.to_string().contains("binding profile"));
        assert!(
            error
                .errors()
                .iter()
                .any(|e| e.message.contains("omit all three"))
        );
    }
    // Same schema, different RPC encoding: diagnostics must distinguish them.
    let mapped_schema = analyze_schema(&parse_schema(
        "version 1; message Request @id(2) { optional uint32 op @id(1); } message Response @id(3) { optional uint32 op @id(1); optional int32 status @id(2); }"
    ).unwrap()).unwrap();
    let source = "profile version 1; rpc Execute { request=Request; response=Response; request_delivery=reliable; response_delivery=reliable; }";
    let managed =
        analyze_binding_profile(&parse_binding_profile(source).unwrap(), &mapped_schema).unwrap();
    let mapped = source.replace("request=Request;", "request=Request; request_operation_id=op; response_operation_id=op; response_status=status;");
    let mapped =
        analyze_binding_profile(&parse_binding_profile(&mapped).unwrap(), &mapped_schema).unwrap();
    assert_ne!(
        binding_profile_identity(&managed),
        binding_profile_identity(&mapped)
    );
}

#[test]
fn managed_metadata_counts_toward_one_frame_limits() {
    for (count, available) in [(508, true), (509, false)] {
        let source = format!(
            "version 1; message Request @id(2) {{ required packed fixed32 values[{count}] @id(1); }} message Response @id(3) {{}}"
        );
        let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let profile = analyze_binding_profile(
            &parse_binding_profile(PROFILE.split("rpc Auxiliary").next().unwrap()).unwrap(),
            &schema,
        )
        .unwrap();
        let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
        assert!(runtime.header.contains(&format!(
            "#define DEMO_HAS_DEFAULT_ENDPOINT {}",
            u8::from(available)
        )));
    }
}

#[test]
fn managed_generated_names_cannot_shadow_schema_or_runtime_members() {
    for declaration in [
        "DemoExecuteCall",
        "DemoExecuteResult",
        "DemoExecuteRequestToken",
    ] {
        let source = format!("{SCHEMA}\nmessage {declaration} @id(30) {{}}");
        let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let profile =
            analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
        assert!(generate_runtime_c(&schema, &profile, "demo").is_err());
    }
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let source = PROFILE.replace("rpc Execute", "rpc rpc_incarnation");
    let profile =
        analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema).unwrap();
    assert!(generate_runtime_c(&schema, &profile, "demo").is_err());
}

#[test]
fn managed_rpc_runs_real_core_and_shared_codec_for_all_deliveries() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned()
        });
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    for request in ["reliable", "unreliable"] {
        for response in ["reliable", "unreliable"] {
            let source = PROFILE
                .replacen(
                    "request_delivery = reliable",
                    &format!("request_delivery = {request}"),
                    1,
                )
                .replacen(
                    "response_delivery = reliable",
                    &format!("response_delivery = {response}"),
                    1,
                );
            let profile =
                analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema).unwrap();
            let codec = generate_c(&schema, "demo").unwrap();
            let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
            let peer = generate_runtime_c_named(&schema, &profile, "demo", "peer").unwrap();
            let temp = tempdir().unwrap();
            for (name, text) in [
                ("demo.h", codec.header), ("demo.c", codec.source),
                ("demo_bindings.h", codec.bindings_header), ("demo_bindings.c", codec.bindings_source),
                ("demo_runtime.h", runtime.header), ("demo_runtime.c", runtime.source),
                ("peer_runtime.h", peer.header), ("peer_runtime.c", peer.source),
                ("test.c", include_str!("fixtures/managed_rpc.c").to_owned()),
                ("headers.cpp", "#include \"demo_runtime.h\"\n#include \"peer_runtime.h\"\nstatic demo_endpoint_t a;\nstatic peer_endpoint_t b;\nint main() { return demo_endpoint_init(&a, 1) + peer_endpoint_init(&b, 2); }\n".to_owned()),
            ] { fs::write(temp.path().join(name), text).unwrap(); }
            let mut cc = Command::new("cc");
            cc.args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
                .arg(format!(
                    "-DREQUEST_RELIABLE={}",
                    u8::from(request == "reliable")
                ))
                .arg(format!(
                    "-DRESPONSE_RELIABLE={}",
                    u8::from(response == "reliable")
                ))
                .arg("-I")
                .arg(root.join("include"))
                .arg("-I")
                .arg(temp.path());
            let mut sources = fs::read_dir(root.join("src"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
                .collect::<Vec<_>>();
            sources.sort();
            cc.args(sources)
                .arg(root.join("adapters/loopback/src/loopback.c"));
            for file in [
                "demo.c",
                "demo_bindings.c",
                "demo_runtime.c",
                "peer_runtime.c",
                "test.c",
            ] {
                cc.arg(temp.path().join(file));
            }
            let executable = temp.path().join("managed-test");
            if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                cc.args([
                    "-fsanitize=address,undefined",
                    "-fno-omit-frame-pointer",
                    "-g",
                ]);
            }
            let result = cc.arg("-o").arg(&executable).output().unwrap();
            assert!(
                result.status.success(),
                "{request}/{response}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let result = Command::new(executable).output().unwrap();
            assert!(
                result.status.success(),
                "{request}/{response}: {}",
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
    }
}

#[test]
fn managed_and_mapped_services_share_one_runtime_without_encoding_scratch_leakage() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let source = format!(
        "{SCHEMA}\nmessage LegacyRequest @id(10) {{ optional uint32 op @id(1); }}\nmessage LegacyResponse @id(11) {{ optional uint32 op @id(1); optional int32 status @id(2); }}"
    );
    let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
    let source = format!(
        "{PROFILE}\nrpc Legacy {{ request=LegacyRequest; response=LegacyResponse; request_operation_id=op; response_operation_id=op; response_status=status; request_delivery=reliable; response_delivery=unreliable; }}"
    );
    let profile =
        analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "demo").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
    assert!(runtime.header.contains("legacy_request_t legacy_request;"));
    assert!(!runtime.header.contains("request_t execute_request;"));
    let temp = tempdir().unwrap();
    for (name, text) in [
        ("demo.h", codec.header),
        ("demo.c", codec.source),
        ("demo_bindings.h", codec.bindings_header),
        ("demo_bindings.c", codec.bindings_source),
        ("demo_runtime.h", runtime.header),
        ("demo_runtime.c", runtime.source),
    ] {
        fs::write(temp.path().join(name), text).unwrap();
    }
    let result = Command::new("cc")
        .args([
            "-std=c11",
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
        .arg(temp.path().join("demo.c"))
        .arg(temp.path().join("demo_runtime.c"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
