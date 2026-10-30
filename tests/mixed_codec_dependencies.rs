// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_c_for_profile,
    generate_runtime_c, parse_binding_profile, parse_schema,
};

#[test]
fn positional_routes_preserve_shared_schema_v1_dependencies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    // Both ID orders, plus a parent without its own route: nested wire policy
    // must never depend on the ordering or existence of top-level routes.
    for (child_id, parent_id) in [(1, 2), (2, 1)] {
        for packed in [false, true] {
            let extra = if packed {
                "required packed fixed32 words[2] @id(2);"
            } else {
                ""
            };
            let schema = analyze_schema(
                &parse_schema(&format!(
                    "version 1;
message Child @id({child_id}) {{ required fixed32 value @id(1); {extra} }}
message Parent @id({parent_id}) {{ required Child child @id(1); }}
message Grandparent @id(3) {{ required Parent parent @id(1); }}
message Reply @id(4) {{}}
message OtherReply @id(5) {{}}"
                ))
                .unwrap(),
            )
            .unwrap();
            for route in ["send", "direct", "latest", "fifo"] {
                for parent_route in ["", "send Parent { delivery=unreliable; codec=schema_v1; }"] {
                    let profile = analyze_binding_profile(
                        &parse_binding_profile(&format!(
                            "profile version 1;
{route} Child {{ delivery=unreliable; codec=positional_v1; }}
{parent_route}
rpc FingerprintGrandparent {{ request=Grandparent; response=OtherReply; }}"
                        ))
                        .unwrap(),
                        &schema,
                    )
                    .unwrap();
                    let reversed = analyze_binding_profile(
                        &parse_binding_profile(&format!(
                            "profile version 1;
rpc FingerprintGrandparent {{ request=Grandparent; response=OtherReply; }}
{parent_route}
{route} Child {{ delivery=unreliable; codec=positional_v1; }}"
                        ))
                        .unwrap(),
                        &schema,
                    )
                    .unwrap();
                    assert_eq!(profile, reversed, "binding order must not select a codec");
                    let dir = tempfile::tempdir().unwrap();
                    let label = format!("child={child_id}/packed={packed}/{route}/{parent_route}");
                    for profiled in [false, true] {
                        let c = if profiled {
                            generate_c_for_profile(&schema, &profile, "mixed").unwrap()
                        } else {
                            generate_c(&schema, "mixed").unwrap()
                        };
                        if profiled {
                            assert_eq!(
                                c,
                                generate_c_for_profile(&schema, &profile, "mixed").unwrap()
                            );
                            let runtime = generate_runtime_c(&schema, &profile, "mixed").unwrap();
                            fs::write(dir.path().join("mixed_runtime.h"), runtime.header).unwrap();
                            fs::write(dir.path().join("mixed_advanced.h"), runtime.advanced_header)
                                .unwrap();
                            fs::write(dir.path().join("mixed_runtime.c"), runtime.source).unwrap();
                        }
                        for (file, data) in [
                            ("mixed.h", c.header),
                            ("mixed_values.h", c.values_header),
                            ("mixed.c", c.source),
                            ("mixed_bindings.h", c.bindings_header),
                            ("mixed_bindings.c", c.bindings_source),
                            (
                                "test.c",
                                include_str!("fixtures/mixed_codec_dependencies.c").into(),
                            ),
                        ] {
                            fs::write(dir.path().join(file), data).unwrap();
                        }
                        let mut cc = Command::new("cc");
                        cc.args([
                            "-std=c11",
                            "-O2",
                            "-Wall",
                            "-Wextra",
                            "-Wpedantic",
                            "-Werror",
                        ])
                        .arg(format!("-DPACKED={}", u8::from(packed)))
                        .arg(format!("-DPOSITIONAL={}", u8::from(profiled)))
                        .arg("-I")
                        .arg(root.join("include"))
                        .arg("-I")
                        .arg(dir.path());
                        if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                            cc.args([
                                "-fsanitize=address,undefined",
                                "-fno-omit-frame-pointer",
                                "-g",
                            ]);
                        }
                        let binary = dir.path().join("test");
                        let result = cc
                            .arg(dir.path().join("test.c"))
                            .arg("-o")
                            .arg(&binary)
                            .output()
                            .unwrap();
                        assert!(
                            result.status.success(),
                            "{label}/profiled={profiled}: {}",
                            String::from_utf8_lossy(&result.stderr)
                        );
                        let result = Command::new(binary).output().unwrap();
                        assert!(
                            result.status.success(),
                            "{label}/profiled={profiled}: {}",
                            String::from_utf8_lossy(&result.stderr)
                        );
                        if profiled {
                            // Compile the matched runtime and bindings too, including
                            // their per-route source/header codec checks.
                            for file in ["mixed_runtime.c", "mixed_bindings.c"] {
                                let result = Command::new("cc")
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
                                    .arg("-I")
                                    .arg(dir.path())
                                    .arg(dir.path().join(file))
                                    .output()
                                    .unwrap();
                                assert!(
                                    result.status.success(),
                                    "{label}/{file}: {}",
                                    String::from_utf8_lossy(&result.stderr)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn an_unbounded_parent_does_not_emit_unused_owned_dependencies() {
    let schema = analyze_schema(
        &parse_schema(
            "version 1;
message Child @id(1) { required fixed32 value @id(1); }
message Parent @id(2) { required Child child @id(1); repeated uint32 extra @id(2); }",
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(
            "profile version 1;
send Child { delivery=unreliable; codec=positional_v1; }
send Parent { delivery=unreliable; }",
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let c = generate_c_for_profile(&schema, &profile, "mixed").unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("mixed.h", c.header),
        ("mixed_values.h", c.values_header),
        ("mixed.c", c.source),
    ] {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let result = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-c",
            "-I",
        ])
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("include"),
        )
        .arg(dir.path().join("mixed.c"))
        .arg("-o")
        .arg(dir.path().join("mixed.o"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn managed_methods_preserve_shared_tagged_dependencies_in_both_service_orders() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    for (child_name, parent_name) in [("AChild", "ZParent"), ("ZChild", "AParent")] {
        for guarantee in ["replay", "correlate"] {
            let schema = analyze_schema(
                &parse_schema(
                    "version 1;
message Child @id(1) { required fixed32 value @id(1); optional uint32 extra @id(2) [default=19]; }
message Parent @id(2) { required Child child @id(1); }
message Reply @id(3) {}
message OtherReply @id(4) {}",
                )
                .unwrap(),
            )
            .unwrap();
            let profile = analyze_binding_profile(
                &parse_binding_profile(&format!(
                    "profile version 1;
rpc {child_name} {{ request=Reply; response=Child; codec=positional_v1; guarantee={guarantee}; }}
rpc {parent_name} {{ request=Parent; response=OtherReply; guarantee={guarantee}; }}"
                ))
                .unwrap(),
                &schema,
            )
            .unwrap();
            let c = generate_c_for_profile(&schema, &profile, "mixed").unwrap();
            let rt = generate_runtime_c(&schema, &profile, "mixed").unwrap();
            let dir = tempfile::tempdir().unwrap();
            for (name, text) in [
                ("mixed.h", c.header),
                ("mixed_values.h", c.values_header),
                ("mixed.c", c.source),
                ("mixed_bindings.h", c.bindings_header),
                ("mixed_bindings.c", c.bindings_source),
                ("mixed_runtime.h", rt.header),
                ("mixed_advanced.h", rt.advanced_header),
                ("mixed_runtime.c", rt.source),
                (
                    "test.c",
                    include_str!("fixtures/mixed_rpc_dependencies.c").into(),
                ),
            ] {
                fs::write(dir.path().join(name), text).unwrap();
            }
            let binary = dir.path().join("test");
            let mut cc = Command::new("cc");
            cc.args([
                "-std=c11",
                "-O2",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(root.join("include"))
            .arg("-I")
            .arg(dir.path());
            cc.arg(format!("-DREPLAY={}", u8::from(guarantee == "replay")));
            if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                cc.args([
                    "-fsanitize=address,undefined",
                    "-fno-omit-frame-pointer",
                    "-g",
                ]);
            }
            let mut sources = fs::read_dir(root.join("src"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
                .collect::<Vec<_>>();
            sources.sort();
            let result = cc
                .arg(dir.path().join("test.c"))
                .arg(dir.path().join("mixed.c"))
                .arg(dir.path().join("mixed_bindings.c"))
                .args(sources)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{child_name}/{guarantee}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let result = Command::new(binary).output().unwrap();
            assert!(
                result.status.success(),
                "{child_name}/{guarantee}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
