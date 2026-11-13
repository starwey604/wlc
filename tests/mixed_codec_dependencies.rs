// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c_for_profile, parse_binding_profile,
    parse_schema,
};
#[test]
fn positional_routes_preserve_shared_tagged_dependencies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    for (child, parent) in [(1, 2), (2, 1)] {
        for packed in [false, true] {
            for route in ["send", "direct"] {
                for parent_route in ["", "send Parent { delivery=unreliable; }"] {
                    let extra = if packed {
                        "required packed fixed32 words[2] @id(2);"
                    } else {
                        ""
                    };
                    let schema=analyze_schema(&parse_schema(&format!("version 1; message Child @id({child}) {{ required fixed32 value @id(1); {extra} }} message Parent @id({parent}) {{ required Child child @id(1); }} message Grandparent @id(3) {{ required Parent parent @id(1); }}")).unwrap()).unwrap();
                    let text = format!(
                        "profile version 2; {route} Child {{ delivery=unreliable; codec=positional_v1; }} {parent_route}"
                    );
                    let profile =
                        analyze_binding_profile(&parse_binding_profile(&text).unwrap(), &schema)
                            .unwrap();
                    let dir = tempfile::tempdir().unwrap();
                    for positional in [false, true] {
                        let c = if positional {
                            generate_c_for_profile(&schema, &profile, "mixed").unwrap()
                        } else {
                            wlc::generate_c(&schema, "mixed").unwrap()
                        };
                        for (name, contents) in [
                            ("mixed.h", c.header),
                            ("mixed_values.h", c.values_header),
                            ("mixed.c", c.source),
                        ] {
                            fs::write(dir.path().join(name), contents).unwrap();
                        }
                        fs::write(
                            dir.path().join("test.c"),
                            include_str!("fixtures/mixed_codec_dependencies.c"),
                        )
                        .unwrap();
                        let mut cc = Command::new(std::env::var("CC").unwrap_or("cc".into()));
                        cc.args([
                            "-std=c11",
                            "-O2",
                            "-Wall",
                            "-Wextra",
                            "-Wpedantic",
                            "-Werror",
                        ]);
                        cc.arg(format!("-DPOSITIONAL={}", u8::from(positional)))
                            .arg(format!("-DPACKED={}", u8::from(packed)));
                        if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                            cc.args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]);
                        }
                        let out = cc
                            .arg("-I")
                            .arg(root.join("include"))
                            .arg("-I")
                            .arg(dir.path())
                            .arg(dir.path().join("test.c"))
                            .arg("-o")
                            .arg(dir.path().join("test"))
                            .output()
                            .unwrap();
                        assert!(
                            out.status.success(),
                            "{}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        assert!(
                            Command::new(dir.path().join("test"))
                                .status()
                                .unwrap()
                                .success()
                        );
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
            "profile version 2;
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
