// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, binding_profile_identity, compose_binding_profiles,
    generate_c, generate_runtime_c_named, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = "version 1; message Request @id(1) { required bytes<31> data @id(1); } message Response @id(2) { required bytes<1970> data @id(1); } message State @id(3) { required uint32 seq @id(1); }";
const RPC: &str = "profile version 1; rpc Echo { request = Request; response = Response; } latest State { delivery = unreliable; }";

#[test]
fn endpoint_declarations_are_local_composable_and_unambiguous() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let resolve =
        |s: &str| analyze_binding_profile(&parse_binding_profile(s).unwrap(), &schema).unwrap();
    let shared = resolve(RPC);
    let client =
        resolve("profile version 1; endpoint { rpc_role = client; envelope = native_packet; }");
    let first = compose_binding_profiles(&[shared.clone(), client.clone()]).unwrap();
    let second = compose_binding_profiles(&[client.clone(), shared.clone()]).unwrap();
    assert_eq!(first, second);
    assert!(first.has_rpc_client() && !first.has_rpc_server());
    assert_ne!(
        binding_profile_identity(&first),
        binding_profile_identity(&shared)
    );
    let defaults = resolve("profile version 1; endpoint { envelope = any; rpc_role = both; }");
    assert_eq!(
        binding_profile_identity(&compose_binding_profiles(&[defaults, shared.clone()]).unwrap()),
        binding_profile_identity(&shared)
    );
    assert!(compose_binding_profiles(&[client.clone(), client]).is_err());
    for body in [
        "endpoint {} endpoint {}",
        "endpoint { rpc_role = client; rpc_role = server; }",
        "endpoint { transport = udp; }",
        "endpoint { envelope = native_packet }",
    ] {
        assert!(
            parse_binding_profile(&format!("profile version 1; {body}")).is_err(),
            "{body}"
        );
    }
    for body in ["rpc_role = none;", "envelope = udp;"] {
        let parsed =
            parse_binding_profile(&format!("profile version 1; endpoint {{ {body} }}")).unwrap();
        assert!(analyze_binding_profile(&parsed, &schema).is_err());
    }
}

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn role_layouts_also_compile_for_mapped_and_unbounded_expert_runtimes() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    for (source, rpc) in [
        (
            "version 1; message Request @id(1) { required uint32 id @id(1); } message Response @id(2) { required uint32 id @id(1); required int32 status @id(2); }",
            "request_operation_id = id; response_operation_id = id; response_status = status;",
        ),
        (
            "version 1; message Request @id(1) { repeated uint32 data @id(1); } message Response @id(2) { required bytes data @id(1); }",
            "",
        ),
    ] {
        let schema = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let codec = generate_c(&schema, "business").unwrap();
        for (name, text) in [
            ("business.h", codec.header),
            ("business_values.h", codec.values_header),
            ("business_bindings.h", codec.bindings_header),
        ] {
            fs::write(temp.path().join(name), text).unwrap();
        }
        for role in ["client", "server", "both"] {
            let text = format!(
                "profile version 1; endpoint {{ rpc_role = {role}; }} rpc Echo {{ request = Request; response = Response; {rpc} }}"
            );
            let profile =
                analyze_binding_profile(&parse_binding_profile(&text).unwrap(), &schema).unwrap();
            let generated = generate_runtime_c_named(&schema, &profile, "business", role).unwrap();
            for (suffix, text) in [
                ("runtime.h", generated.header),
                ("runtime.c", generated.source),
                ("endpoint.h", generated.endpoint_header),
                ("advanced.h", generated.advanced_header),
            ] {
                fs::write(temp.path().join(format!("{role}_{suffix}")), text).unwrap();
            }
            run(Command::new("cc")
                .args([
                    "-std=c11",
                    "-Wall",
                    "-Wextra",
                    "-Wpedantic",
                    "-Werror",
                    "-fsyntax-only",
                    "-I",
                ])
                .arg(root.join("include"))
                .arg(temp.path().join(format!("{role}_runtime.c"))));
            fs::write(
                temp.path().join("headers.cpp"),
                format!("#include \"{role}_advanced.h\"\n"),
            )
            .unwrap();
            run(Command::new("c++")
                .args([
                    "-std=c++20",
                    "-Wall",
                    "-Wextra",
                    "-Wpedantic",
                    "-Werror",
                    "-fsyntax-only",
                    "-I",
                ])
                .arg(root.join("include"))
                .arg(temp.path().join("headers.cpp")));
        }
    }
}

#[test]
fn trimmed_layouts_run_with_all_envelopes_and_slot_capacities() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let mut core = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect::<Vec<_>>();
    core.sort();
    for envelope in ["native_packet", "cobs_stream", "bus_length16"] {
        let temp = tempfile::tempdir().unwrap();
        let codec = generate_c(&schema, "business").unwrap();
        for (name, text) in [
            ("business.h", codec.header),
            ("business.c", codec.source),
            ("business_values.h", codec.values_header),
            ("business_bindings.h", codec.bindings_header),
            ("business_bindings.c", codec.bindings_source),
        ] {
            fs::write(temp.path().join(name), text).unwrap();
        }
        let mut headers = String::new();
        for role in ["client", "server", "both", "flex"] {
            let source = if role == "flex" {
                RPC.to_owned()
            } else {
                format!("{RPC} endpoint {{ envelope = {envelope}; rpc_role = {role}; }}")
            };
            let profile =
                analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema).unwrap();
            let generated = generate_runtime_c_named(&schema, &profile, "business", role).unwrap();
            if role == "server" {
                assert!(!generated.header.contains("uint8_t requests["));
                assert!(!generated.header.contains("client_slot_t rpc_client_slot;"));
                assert!(!generated.header.contains("server_endpoint_echo_async("));
            }
            if role == "client" {
                assert!(!generated.header.contains("request_value_t request;"));
                assert!(
                    !generated
                        .header
                        .contains("server_cache_slot_t rpc_server_cache_slot;")
                );
                assert!(!generated.header.contains(" on_echo;"));
            }
            if envelope != "cobs_stream" && role != "flex" {
                assert!(!generated.header.contains("uint8_t rx_fifo["));
            }
            for (suffix, text) in [
                ("runtime.h", generated.header),
                ("runtime.c", generated.source),
                ("endpoint.h", generated.endpoint_header),
                ("advanced.h", generated.advanced_header),
            ] {
                fs::write(temp.path().join(format!("{role}_{suffix}")), text).unwrap();
            }
            headers.push_str(&format!("#include \"{role}_advanced.h\"\n"));
        }
        fs::write(temp.path().join("headers.cpp"), headers).unwrap();
        run(Command::new("c++")
            .args([
                "-std=c++20",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-fsyntax-only",
                "-I",
            ])
            .arg(root.join("include"))
            .arg(temp.path().join("headers.cpp")));
        fs::write(
            temp.path().join("test.c"),
            include_str!("fixtures/endpoint_layout.c"),
        )
        .unwrap();
        for capacity in [1, 4] {
            let mut cc = Command::new("cc");
            cc.args([
                "-std=c11",
                "-O1",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(root.join("include"))
            .args(["-I"])
            .arg(temp.path());
            for role in ["CLIENT", "SERVER", "BOTH", "FLEX"] {
                cc.arg(format!("-D{role}_ENDPOINT_RPC_CAPACITY={capacity}"));
            }
            if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                cc.args([
                    "-fsanitize=address,undefined",
                    "-fno-omit-frame-pointer",
                    "-g",
                ]);
            }
            cc.args(&core);
            for file in [
                "business.c",
                "business_bindings.c",
                "client_runtime.c",
                "server_runtime.c",
                "both_runtime.c",
                "flex_runtime.c",
                "test.c",
            ] {
                cc.arg(temp.path().join(file));
            }
            let binary = temp.path().join("layout-test");
            run(cc.arg("-o").arg(&binary));
            print!(
                "{envelope} slots={capacity}: {}",
                run(&mut Command::new(binary))
            );
        }
    }
}
