// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c_for_profile, parse_binding_profile,
    parse_schema,
};
const SCHEMA: &str = "version 1;
enum Mode @id(20) { ZERO = 0; ONE = 1; }
message Request @id(1) { required fixed32 a @id(1); required fixed32 b @id(2); optional fixed32 c @id(3) [default=19]; }
message Response @id(2) { required fixed32 result @id(1); }
message Values @id(3) {
 required bool flag @id(1); required uint8 u8 @id(2); required uint16 u16 @id(3); required uint32 u32 @id(4); required uint64 u64 @id(5);
 required int8 i8 @id(6); required int16 i16 @id(7); required int32 i32 @id(8); required int64 i64 @id(9); required Mode mode @id(10);
 required fixed32 f32 @id(11); required fixed64 f64 @id(12); required float32 x @id(13); required float64 y @id(14);
 required packed fixed32 words[2] @id(15); packed float64 floats[2] @id(16); required packed fixed64 longs[2] @id(17);
}
message Bits @id(4) { optional bool a @id(1); optional bool b @id(2); optional bool c @id(3); optional bool d @id(4); optional bool e @id(5); optional bool f @id(6); optional bool g @id(7); optional bool h @id(8); optional bool i @id(9); }
message Empty @id(5) {}
message Small @id(10) { optional float32 x @id(1); required float64 y @id(2); required fixed64 z @id(3); }
message Narrow @id(7) { required uint8 v @id(1); }
message Wide @id(8) { required uint64 v @id(1); }
message OpenEnum @id(9) { required Mode v @id(1); }
message RpcFloat @id(11) { required float32 x @id(1); required float64 y @id(2); required packed float64 floats[2] @id(3); }
message RpcFloatReply @id(12) {}
message Sample @id(6) { required packed float32 samples[2] @id(1); required fixed32 sequence @id(2); }";

#[test]
fn positional_public_vectors_preserve_the_full_legal_value_domain() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let mut text = String::from("profile version 2; endpoint { envelope=native_packet; }");
    for name in [
        "Request", "Values", "Bits", "Empty", "Small", "Narrow", "Wide", "OpenEnum", "Sample",
    ] {
        text.push_str(&format!(
            "send {name} {{ delivery=unreliable; codec=positional_v1; }}"
        ));
    }
    text.push_str("rpc FloatProbe { request=RpcFloat; response=RpcFloatReply; }");
    let profile = analyze_binding_profile(&parse_binding_profile(&text).unwrap(), &schema).unwrap();
    let kernel = wlc::KernelProfile::from_bindings(profile.clone()).unwrap();
    let c = wlc::generate_kernel_c(&schema, &kernel, "demo").unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in c.files {
        fs::write(dir.path().join(name), contents).unwrap();
    }
    fs::write(
        dir.path().join("test.c"),
        include_str!("fixtures/positional.c"),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut cc = Command::new(std::env::var("CC").unwrap_or("cc".into()));
    cc.args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]);
    }
    let out = cc
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("demo.c"))
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
    let old = wlc::generate_c(&schema, "demo").unwrap();
    fs::write(dir.path().join("demo.c"), old.source).unwrap();
    let out = Command::new("cc")
        .args(["-std=c11", "-fsyntax-only", "-I"])
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("demo.c"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Codec source/header profile mismatch"));
    assert_eq!(
        generate_c_for_profile(&schema, &profile, "demo").unwrap(),
        generate_c_for_profile(&schema, &profile, "demo").unwrap()
    );
}
