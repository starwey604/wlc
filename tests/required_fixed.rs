// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

#[test]
fn required_fixed_encoders_preserve_wire_preflight_and_bit_patterns() {
    let model = analyze_schema(
        &parse_schema(
            "version 1;
        message Sample @id(1) {
          required fixed32 sequence @id(1); required fixed32 tick @id(2);
          required fixed32 value @id(3); required fixed32 flags @id(4);
        }
        message Wide @id(2) {
          required fixed32 a @id(1); required fixed64 b @id(16);
          required float32 c @id(2048); required float64 d @id(65535);
        }
        message Twin @id(3) {
          required fixed32 first @id(1); required fixed64 second @id(16);
          required float32 third @id(2048); required float64 fourth @id(65535);
        }
        message Optional @id(4) { required fixed32 a @id(1); optional fixed32 b @id(2); }
        message Variable @id(5) { required uint32 a @id(1); required fixed32 b @id(2); }",
        )
        .unwrap(),
    )
    .unwrap();
    let c = generate_c(&model, "required_fixed").unwrap();
    let dir = tempdir().unwrap();
    for (name, text) in [
        ("required_fixed.h", c.header),
        ("required_fixed_values.h", c.values_header),
        ("required_fixed.c", c.source),
        ("test.c", include_str!("fixtures/required_fixed.c").into()),
    ] {
        fs::write(dir.path().join(name), text).unwrap();
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
        let binary = dir.path().join(compiler);
        let mut command = Command::new(compiler);
        command.args([
            "-std=c11",
            "-Os",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
        ]);
        if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
            command.args([
                "-fsanitize=address,undefined",
                "-fno-omit-frame-pointer",
                "-g",
            ]);
        }
        let result = command
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
            "{compiler}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{compiler}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
