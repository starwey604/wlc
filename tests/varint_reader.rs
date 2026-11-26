// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{analyze_schema, generate_c, parse_schema};
use wlc::{codegen::generate_c_for_nodes, node::analyze_nodes};

#[test]
fn readers_preserve_canonical_errors_cursor_and_partial_decode_state() {
    for mode in ["schema_v1", "positional_v1"] {
        let schema = if mode == "schema_v1" {
            // The nested use supplies a generic descriptor for the independent reference.
            "version 1; message StreamChunk @id(103) { required fixed32 sequence @id(1); required bytes<1024> data @id(2); } message Envelope @id(104) { StreamChunk chunk @id(1); }"
        } else {
            "version 1; message Scalar @id(1) { required uint64 value @id(1); }"
        };
        let model = analyze_schema(&parse_schema(schema).unwrap()).unwrap();
        let generate = || {
            if mode == "schema_v1" {
                generate_c(&model, "candidate").unwrap()
            } else {
                let ast = parse_schema(&format!(
                    "{schema} node test {{ recv Scalar @codec(positional_v1); }}"
                ))
                .unwrap();
                let nodes = analyze_nodes(&ast, &model).unwrap();
                generate_c_for_nodes(&model, &nodes, "candidate", false).unwrap()
            }
        };
        let generated = generate();
        assert_eq!(
            generated,
            generate(),
            "{mode} generation must be deterministic"
        );
        let dir = tempfile::tempdir().unwrap();
        for (name, contents) in [
            ("candidate.c", generated.source),
            ("candidate.h", generated.header),
            (
                "varint_reference.c",
                include_str!("fixtures/varint_reference.c").into(),
            ),
            ("test.c", include_str!("fixtures/varint_contract.c").into()),
        ] {
            fs::write(dir.path().join(name), contents).unwrap();
        }
        let root = std::env::var_os("WIRELINK_SOURCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .into()
            });
        for compiler in ["gcc", "clang"] {
            let mut cc = Command::new(compiler);
            cc.args([
                "-std=c11",
                "-Os",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
            ]);
            if mode != "schema_v1" {
                cc.arg("-DREADER_ONLY");
            }
            if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                cc.args([
                    "-fsanitize=address,undefined",
                    "-fno-omit-frame-pointer",
                    "-g",
                ]);
            }
            let binary = dir.path().join(compiler);
            let output = cc
                .arg("-I")
                .arg(root.join("include"))
                .arg("-I")
                .arg(dir.path())
                .arg(root.join("src/codec.c"))
                .arg(dir.path().join("test.c"))
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{mode}/{compiler}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new(binary).output().unwrap();
            assert!(
                output.status.success(),
                "{mode}/{compiler}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            println!(
                "{mode}/{compiler}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
