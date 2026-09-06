use std::{fs, path::PathBuf, process::Command};

use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

const SCHEMA: &str = r#"
version 1;
enum State @id(1) { READY = 0; STOPPED = 1; }
message DeviceInfo @id(2) {
  required string<31> name @id(1);
  optional string<3> greeting @id(2) [default = "电"];
  optional bytes<4> serial @id(3);
  optional State state @id(4) [default = 1];
}
message Snapshot @id(3) {
  required DeviceInfo device @id(1);
  required packed float32 samples[3] @id(2);
  optional DeviceInfo backup @id(3);
}
message Large @id(4) { required bytes<2031> payload @id(1); }
message Unbounded @id(5) { repeated DeviceInfo devices @id(1); }
message Empty @id(6) {}
"#;

#[test]
fn bounded_values_own_nested_data_and_keep_wire_semantics() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let model = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let generated = generate_c(&model, "owned").unwrap();
    assert_eq!(generated, generate_c(&model, "owned").unwrap());
    assert!(
        generated
            .values_header
            .contains("#define UNBOUNDED_HAS_VALUE 0")
    );
    assert!(!generated.header.contains("unbounded_value_t"));
    let directory = tempdir().unwrap();
    for (file, text) in [
        ("owned.h", generated.header),
        ("owned_values.h", generated.values_header),
        ("owned.c", generated.source),
        ("test.c", include_str!("fixtures/owned_values.c").to_owned()),
        (
            "headers.cpp",
            r#"#include "owned_values.h"
#include <type_traits>
static_assert(std::is_trivially_copyable_v<snapshot_value_t>);
static_assert(std::is_standard_layout_v<snapshot_value_t>);
int main() { snapshot_value_t a{}, b{}; a = b; return int(a.device.name.length); }
"#
            .to_owned(),
        ),
    ] {
        fs::write(directory.path().join(file), text).unwrap();
    }
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
        "-fstack-usage",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    if std::env::var_os("WLC_TEST_BENCHMARK").is_some() {
        cc.arg("-DWLC_VALUE_BENCHMARK");
    }
    let binary = directory.path().join("test");
    let result = cc
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("owned.c"))
        .arg(directory.path().join("test.c"))
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
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    print!("{}", String::from_utf8_lossy(&result.stdout));
    for file in fs::read_dir(directory.path()).unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "su") {
            for line in fs::read_to_string(path).unwrap().lines() {
                if line.contains("_value_") {
                    println!("stack {}", line.rsplit_once(':').unwrap().1);
                }
            }
        }
    }
    // A binding owns a normal business value, not an endpoint/slot layout.
    fs::write(
        directory.path().join("ffi.py"),
        include_str!("fixtures/owned_values.py"),
    )
    .unwrap();
    let library = directory.path().join("owned.so");
    let result = Command::new("cc")
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fPIC",
            "-shared",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("owned.c"))
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new("python3")
        .arg(directory.path().join("ffi.py"))
        .arg(library)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
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
        .arg(directory.path())
        .arg(directory.path().join("headers.cpp"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn owned_symbols_cannot_shadow_schema_types_or_macros() {
    for collision in [
        "message DeviceInfoValue @id(40) {}",
        "enum Clash @id(40) { DEVICE_INFO_VALUE_SIZE = 0; }",
        "enum Clash @id(40) { DEVICE_INFO_HAS_VALUE = 0; }",
    ] {
        let model =
            analyze_schema(&parse_schema(&format!("{SCHEMA}\n{collision}")).unwrap()).unwrap();
        assert!(generate_c(&model, "owned").is_err());
    }
}
