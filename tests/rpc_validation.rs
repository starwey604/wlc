use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn canonical_sink_and_trusted_values_preserve_boundary_contracts() {
    let schema = analyze_schema(
        &parse_schema(
            r#"
version 1;
message Child @id(1) {
  required string<64> text @id(1);
  optional string<3> greeting @id(2) [default = "电"];
  packed float64 weights[4] @id(3);
}
message Request @id(2) {
  required Child child @id(1);
  required bytes<128> body @id(2);
  packed fixed32 samples[16] @id(3);
  optional int64 integer @id(4);
  optional float32 single @id(5);
  optional float64 double_value @id(6);
  optional bool enabled @id(7);
  optional uint64 unsigned_value @id(8) [default = 18446744073709551615];
  optional int8 tiny @id(9);
  optional uint32 default_unsigned @id(10) [default = 4294967295];
  optional int32 default_signed @id(11) [default = -2147483648];
  optional int64 default_wide @id(12) [default = -9223372036854775808];
}
message Reply @id(3) { required uint32 result @id(1); }
message Repeated @id(4) { repeated uint32 items @id(1); repeated Child children @id(2); }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(&parse_binding_profile(
        "profile version 1; rpc Execute { request = Request @delivery(unreliable); response = Reply @delivery(unreliable); }"
    ).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "validation").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "validation").unwrap();
    assert!(!codec.header.contains("wlc_detail"));
    assert!(!codec.values_header.contains("wlc_detail"));
    assert!(!runtime.header.contains("canonical_request"));
    assert!(!runtime.source.contains("request_value_from_view("));
    // Test-only work counters; production generated C contains no probes.
    let source = codec
        .source
        .replace(
            "static bool wlc_utf8(const uint8_t *s, size_t n) {",
            "static bool wlc_utf8(const uint8_t *s, size_t n) { ++utf8_calls;",
        )
        .replace(
            "  wlc_hash_state_t state =",
            "  ++fingerprint_calls;\n  wlc_hash_state_t state =",
        );
    let dir = tempdir().unwrap();
    for (file, text) in [
        ("validation.h", codec.header),
        ("validation_values.h", codec.values_header),
        ("validation.c", source),
        ("validation_bindings.h", codec.bindings_header),
        ("validation_bindings.c", codec.bindings_source),
        ("validation_runtime.h", runtime.header),
        ("validation_runtime.c", runtime.source),
        ("test.c", include_str!("fixtures/rpc_validation.c").into()),
    ] {
        fs::write(dir.path().join(file), text).unwrap();
    }
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let mut command = Command::new("cc");
    command.args([
        "-std=c11",
        "-O2",
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
    let mut sources = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect::<Vec<_>>();
    sources.sort();
    let binary = dir.path().join("test");
    let output = command
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("test.c"))
        .arg(dir.path().join("validation_bindings.c"))
        .args(sources)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}
