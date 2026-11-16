// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{analyze_schema, parse_schema};
use wlc::{codegen::generate_c_for_nodes, node::analyze_nodes};
#[test]
fn positional_routes_preserve_shared_tagged_dependencies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    for (child, parent) in [(1, 2), (2, 1)] {
        for packed in [false, true] {
            for route in ["send", "recv"] {
                for parent_route in ["", "send Parent;"] {
                    let extra = if packed {
                        "required packed fixed32 words[2] @id(2);"
                    } else {
                        ""
                    };
                    let source = format!(
                        "version 1; message Child @id({child}) {{ required fixed32 value @id(1); {extra} }} message Parent @id({parent}) {{ required Child child @id(1); }} message Grandparent @id(3) {{ required Parent parent @id(1); }} node test {{ {route} Child @codec(positional_v1); {parent_route} }}"
                    );
                    let ast = parse_schema(&source).unwrap();
                    let schema = analyze_schema(&ast).unwrap();
                    let nodes = analyze_nodes(&ast, &schema).unwrap();
                    let dir = tempfile::tempdir().unwrap();
                    for positional in [false, true] {
                        let c = if positional {
                            generate_c_for_nodes(&schema, &nodes, "mixed", false).unwrap()
                        } else {
                            wlc::generate_c(&schema, "mixed").unwrap()
                        };
                        for (name, contents) in [("mixed.h", c.header), ("mixed.c", c.source)] {
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
                            .arg(root.join("src/codec.c"))
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
    let ast = parse_schema(
        "version 1;
message Child @id(1) { required fixed32 value @id(1); }
message Parent @id(2) { required Child child @id(1); repeated uint32 extra @id(2); }
node test { send Child @codec(positional_v1); }",
    )
    .unwrap();
    let schema = analyze_schema(&ast).unwrap();
    let nodes = analyze_nodes(&ast, &schema).unwrap();
    let c = generate_c_for_nodes(&schema, &nodes, "mixed", false).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [("mixed.h", c.header), ("mixed.c", c.source)] {
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
