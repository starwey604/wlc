// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

#[test]
fn small_scalar_codecs_match_generic_wire_errors_and_partial_output() {
    let schema = analyze_schema(
        &parse_schema(
            "version 1;
             message Fixed @id(1) {
               required fixed32 a @id(1); optional fixed64 b @id(16) [default = 18446744073709551615];
               required float32 c @id(2048); optional float64 d @id(65535);
             }
             message Var @id(2) {
               required uint32 a @id(1); optional uint32 b @id(16) [default = 4294967295];
               required uint32 c @id(2048); optional uint32 d @id(65535);
             }
             message Mixed @id(3) {
               required uint32 a @id(1); optional fixed32 b @id(16) [default = 4294967295];
               required fixed64 c @id(2048); optional float32 d @id(65535);
             }
             message VarTwin @id(4) {
               required uint32 id @id(1); optional uint32 limit @id(16) [default = 4294967295];
               required uint32 left @id(2048); optional uint32 right @id(65535);
             }
             message Prefix @id(5) {
               required uint32 a @id(1); required uint32 abc @id(2); optional uint32 ab @id(3);
             }
             message WlcFoo @id(6) {
               required fixed32 wlc_foo_encode @id(1);
             }
             message RequestExtra @id(7) {
               optional fixed32 request_extra_encode @id(1); optional fixed32 wlc_clear @id(2);
             }",
        )
        .unwrap(),
    )
    .unwrap();
    let codec = generate_c(&schema, "sample").unwrap();
    for index in 0..6 {
        let start = codec
            .source
            .find(&format!(
                "wl_codec_status_t wlc_scalar_shape_{index}_decode("
            ))
            .unwrap();
        let body = &codec.source[start..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("switch ((uint16_t)number)"));
        assert!(!body.contains("wlc_decode("));
        assert!(!body.contains("wlc_read_value("));
    }
    assert!(!codec.source.contains("wlc_scalar_shape_6_decode("));
    for name in ["var", "var_twin"] {
        let start = codec
            .source
            .find(&format!("wl_codec_status_t {name}_decode("))
            .unwrap();
        assert!(
            codec.source[start..]
                .lines()
                .next()
                .unwrap()
                .contains("wlc_scalar_shape_1_decode(")
        );
    }
    let dir = tempdir().unwrap();
    for (file, contents) in [
        ("sample.h", codec.header),
        ("sample_values.h", codec.values_header),
        ("sample.c", codec.source),
        (
            "test.c",
            include_str!("fixtures/scalar_specialization.c").into(),
        ),
    ] {
        fs::write(dir.path().join(file), contents).unwrap();
    }
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let binary = dir.path().join("test");
    let result = cc
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
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(binary).output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
