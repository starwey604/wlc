// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c_for_profile, generate_runtime_c,
    parse_binding_profile, parse_schema,
};

#[test]
fn replay_correlate_and_mixed_generate_and_run_with_both_metadata_modes_and_deliveries() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let schema = analyze_schema(&parse_schema("version 1;
        message StrongRequest @id(1) { required uint32 value @id(1); optional uint32 op @id(2); }
        message StrongResponse @id(2) { required uint32 value @id(1); optional uint32 op @id(2); optional int32 status @id(3); }
        message LightRequest @id(3) { required uint32 value @id(1); optional uint32 op @id(2); }
        message LightResponse @id(4) { required bytes<256> body @id(1); optional uint32 op @id(2); optional int32 status @id(3); }").unwrap()).unwrap();
    for (mapped, positional) in [(false, false), (false, true), (true, false)] {
        for (strong, light) in [
            ("replay", "replay"),
            ("replay", "correlate"),
            ("correlate", "correlate"),
        ] {
            for reliable in [false, true] {
                let delivery = if reliable { "reliable" } else { "unreliable" };
                let mapping = if mapped {
                    "request_operation_id=op; response_operation_id=op; response_status=status;"
                } else {
                    ""
                };
                let codec = if positional {
                    "codec=positional_v1;"
                } else {
                    ""
                };
                let source = format!("profile version 1;
                  rpc Strong {{request=StrongRequest;response=StrongResponse;guarantee={strong};{codec}request_delivery={delivery};response_delivery={delivery};{mapping}}}
                  rpc Light {{request=LightRequest;response=LightResponse;guarantee={light};request_delivery={delivery};response_delivery={delivery};{mapping}}}");
                let profile =
                    analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema)
                        .unwrap();
                let c = generate_c_for_profile(&schema, &profile, "demo").unwrap();
                let rt = generate_runtime_c(&schema, &profile, "demo").unwrap();
                if strong == "correlate" {
                    for text in [&c.source, &rt.source] {
                        assert!(!text.contains("wlc_detail_fingerprint"));
                        assert!(!text.contains("wlc_hash_"));
                    }
                    assert!(!rt.source.contains("wl_rpc_server_begin("));
                    assert!(!rt.source.contains("wl_rpc_server_poll("));
                    assert!(!rt.header.contains("rpc_server_cache_slot_count"));
                    assert!(!rt.header.contains("rpc_server_cache_ttl_ms"));
                } else if light == "correlate" {
                    assert!(c.source.contains("strong_request_wlc_detail_fingerprint("));
                    assert!(!c.source.contains("light_request_wlc_detail_fingerprint("));
                    let cap = if mapped {
                        18
                    } else if positional {
                        29
                    } else {
                        31
                    };
                    assert!(
                        rt.source
                            .contains(&format!("config->rpc_server_response_capacity = {cap}U"))
                    );
                }
                let dir = tempdir().unwrap();
                for (name, text) in [
                    ("demo.h", c.header),
                    ("demo_values.h", c.values_header),
                    ("demo.c", c.source),
                    ("demo_bindings.h", c.bindings_header),
                    ("demo_bindings.c", c.bindings_source),
                    ("demo_runtime.h", rt.header),
                    ("demo_runtime.c", rt.source),
                    ("demo_advanced.h", rt.advanced_header),
                    (
                        "test.c",
                        include_str!("fixtures/rpc_guarantees.c").to_owned(),
                    ),
                ] {
                    fs::write(dir.path().join(name), text).unwrap();
                }
                let mut cc = Command::new("cc");
                cc.args([
                    "-std=c11",
                    "-Wall",
                    "-Wextra",
                    "-Wpedantic",
                    "-Werror",
                    "-ffunction-sections",
                    "-fdata-sections",
                    "-Wl,--gc-sections",
                ]);
                for (key, value) in [
                    ("MAPPED", mapped),
                    ("POSITIONAL_STRONG", positional),
                    ("REQUEST_RELIABLE", reliable),
                    ("HAS_REPLAY", strong == "replay"),
                    ("HAS_CORRELATE", light == "correlate"),
                    ("LIGHT_CORRELATE", light == "correlate"),
                    ("STRONG_CORRELATE", strong == "correlate"),
                ] {
                    cc.arg(format!("-D{key}={}", u8::from(value)));
                }
                cc.arg("-I")
                    .arg(root.join("include"))
                    .arg("-I")
                    .arg(root.join("tests/support"))
                    .arg("-I")
                    .arg(dir.path());
                let mut core = fs::read_dir(root.join("src"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| p.extension().is_some_and(|e| e == "c"))
                    .collect::<Vec<_>>();
                core.sort();
                cc.args(core);
                for file in ["demo.c", "demo_bindings.c", "demo_runtime.c", "test.c"] {
                    cc.arg(dir.path().join(file));
                }
                if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                    cc.args([
                        "-fsanitize=address,undefined",
                        "-fno-omit-frame-pointer",
                        "-g",
                    ]);
                }
                let exe = dir.path().join("test");
                let output = cc.arg("-o").arg(&exe).output().unwrap();
                assert!(
                    output.status.success(),
                    "{mapped}/{positional}/{strong}/{light}/{delivery}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if strong == "correlate" {
                    let symbols = Command::new("nm").arg(&exe).output().unwrap();
                    assert!(symbols.status.success());
                    let symbols = String::from_utf8(symbols.stdout).unwrap();
                    for line in symbols.lines() {
                        let name = line.split_whitespace().last().unwrap_or("");
                        assert!(
                            ![
                                "wl_rpc_server_begin",
                                "wl_rpc_server_poll",
                                "wl_rpc_server_get_deadline_hint",
                                "server_cache_expire",
                                "server_expire"
                            ]
                            .contains(&name),
                            "retained history symbol: {name}"
                        );
                        assert!(
                            !name.contains("fingerprint") && !name.contains("wlc_hash"),
                            "retained fingerprint: {name}"
                        );
                    }
                }
                let output = Command::new(exe).output().unwrap();
                assert!(
                    output.status.success(),
                    "{mapped}/{positional}/{strong}/{light}/{delivery}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn policy_generation_rejects_the_previous_core_contract() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let schema = analyze_schema(
        &parse_schema("version 1; message Request @id(1) {} message Response @id(2) {}").unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(
            "profile version 1; rpc Query {request=Request;response=Response;guarantee=correlate;}",
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let c = generate_c_for_profile(&schema, &profile, "demo").unwrap();
    let rt = generate_runtime_c(&schema, &profile, "demo").unwrap();
    let d = tempdir().unwrap();
    for (name, value) in [
        ("demo.h", c.header),
        ("demo_values.h", c.values_header),
        ("demo_bindings.h", c.bindings_header),
        ("demo_runtime.h", rt.header),
        ("demo_advanced.h", rt.advanced_header),
    ] {
        fs::write(d.path().join(name), value).unwrap();
    }
    fs::create_dir_all(d.path().join("wirelink")).unwrap();
    let previous = fs::read_to_string(root.join("include/wirelink/version.h"))
        .unwrap()
        .replace(
            "WIRELINK_CODEGEN_CONTRACT_MINOR 12",
            "WIRELINK_CODEGEN_CONTRACT_MINOR 11",
        );
    fs::write(d.path().join("wirelink/version.h"), previous).unwrap();
    fs::write(d.path().join("test.c"), "#include \"demo_runtime.h\"\n").unwrap();
    let result = Command::new("cc")
        .args(["-std=c11", "-fsyntax-only", "-I"])
        .arg(d.path())
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(root.join("runtime/storage/include"))
        .arg(d.path().join("test.c"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("Generated code requires an incompatible Wirelink core")
    );
}
