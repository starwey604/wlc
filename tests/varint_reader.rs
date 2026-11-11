// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_c_for_profile,
    parse_binding_profile, parse_schema,
};

#[test]
fn readers_preserve_canonical_errors_cursor_and_partial_decode_state() {
    for mode in ["schema_v1", "managed_tagged", "positional_v1"] {
        let schema = if mode == "schema_v1" {
            "version 1; message StreamChunk @id(103) { required fixed32 sequence @id(1); required bytes<1024> data @id(2); }"
        } else {
            "version 1; message Scalar @id(1) { required uint64 value @id(1); }"
        };
        let model = analyze_schema(&parse_schema(schema).unwrap()).unwrap();
        let generate = || {
            if mode == "schema_v1" {
                generate_c(&model, "candidate").unwrap()
            } else {
                let codec = if mode == "positional_v1" {
                    "codec=positional_v1;"
                } else {
                    ""
                };
                let profile = analyze_binding_profile(
                    &parse_binding_profile(&format!(
                        "profile version 2; direct Scalar {{ delivery=unreliable; {codec} }}"
                    ))
                    .unwrap(),
                    &model,
                )
                .unwrap();
                generate_c_for_profile(&model, &profile, "candidate").unwrap()
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
            ("candidate_values.h", generated.values_header),
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
