// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c_for_profile, generate_runtime_c,
    parse_binding_profile, parse_schema,
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
fn positional_vectors_and_rpc_guarantees_compile_and_run_without_fallback() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    for guarantee in ["replay", "correlate"] {
        for codec in ["managed_tagged", "positional_v1"] {
            for reliable in [false, true] {
                let delivery = if reliable { "reliable" } else { "unreliable" };
                let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
                let profile = analyze_binding_profile(&parse_binding_profile(&format!("profile version 1;
      rpc Execute {{request=Request;response=Response;guarantee={guarantee};codec={codec};request_delivery={delivery};response_delivery={delivery};}}
      rpc FloatProbe {{request=RpcFloat;response=RpcFloatReply;guarantee={guarantee};codec={codec};request_delivery={delivery};response_delivery={delivery};}}
      send Small {{delivery=unreliable;codec=positional_v1;}} send Narrow {{delivery=unreliable;codec=positional_v1;}} send Wide {{delivery=unreliable;codec=positional_v1;}} send OpenEnum {{delivery=unreliable;codec=positional_v1;}}
      send Values {{delivery=unreliable;codec=positional_v1;}} send Bits {{delivery=unreliable;codec=positional_v1;}} send Empty {{delivery=unreliable;codec=positional_v1;}}
      send Sample {{delivery=unreliable;codec=positional_v1;}} latest Sample {{delivery=unreliable;codec=positional_v1;}}"
    )).unwrap(), &schema).unwrap();
                let c = generate_c_for_profile(&schema, &profile, "demo").unwrap();
                assert_eq!(
                    c,
                    generate_c_for_profile(&schema, &profile, "demo").unwrap()
                );
                let rt = generate_runtime_c(&schema, &profile, "demo").unwrap();
                assert!(!rt.source.contains("demo_rpc_values_value_encode"));
                assert!(rt.header.contains(if codec == "positional_v1" {
                    "managed-rpc-v4-positional-v1"
                } else {
                    "managed-rpc-v4-tagged-varuint32"
                }));
                let dir = tempfile::tempdir().unwrap();
                for (file, data) in [
                    ("demo.h", c.header),
                    ("demo_values.h", c.values_header),
                    ("demo.c", c.source),
                    ("demo_bindings.h", c.bindings_header),
                    ("demo_bindings.c", c.bindings_source),
                    ("demo_runtime.h", rt.header),
                    ("demo_advanced.h", rt.advanced_header),
                    ("demo_runtime.c", rt.source),
                    ("test.c", include_str!("fixtures/positional.c").into()),
                ] {
                    fs::write(dir.path().join(file), data).unwrap();
                }
                let binary = dir.path().join("test");
                let mut cc = Command::new("cc");
                cc.args([
                    "-std=c11",
                    "-O2",
                    "-fstrict-aliasing",
                    "-Wall",
                    "-Wextra",
                    "-Wpedantic",
                    "-Werror",
                ]);
                cc.arg(format!(
                    "-DPOSITIONAL_RPC={}",
                    u8::from(codec == "positional_v1")
                ));
                cc.arg(format!("-DREPLAY={}", u8::from(guarantee == "replay")));
                cc.arg("-I").arg(dir.path());
                for path in [
                    "include",
                    "tests/support",
                    "runtime/storage/include",
                    "adapters/loopback/include",
                ] {
                    cc.arg("-I").arg(root.join(path));
                }
                for path in fs::read_dir(root.join("src"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| p.extension().is_some_and(|e| e == "c"))
                {
                    cc.arg(path);
                }
                for path in [
                    "runtime/storage/src/fixed_pool.c",
                    "adapters/loopback/src/loopback.c",
                ] {
                    cc.arg(root.join(path));
                }
                for path in ["demo.c", "demo_bindings.c", "test.c"] {
                    cc.arg(dir.path().join(path));
                }
                if std::env::var_os("WLC_TEST_X87").is_some() {
                    cc.args(["-mfpmath=387", "-mno-sse"]);
                }
                if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
                    cc.args([
                        "-fsanitize=address,undefined",
                        "-fno-omit-frame-pointer",
                        "-g",
                    ]);
                }
                let out = cc.arg("-o").arg(&binary).output().unwrap();
                assert!(
                    out.status.success(),
                    "{guarantee}/{codec}/{delivery}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let out = Command::new(&binary).output().unwrap();
                assert!(
                    out.status.success(),
                    "{guarantee}/{codec}/{delivery}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                // An unprofiled Schema-v1 codec must fail when paired with positional routes.
                let old = wlc::generate_c(&schema, "demo").unwrap();
                fs::write(dir.path().join("demo.c"), &old.source).unwrap();
                let mismatch = Command::new("cc")
                    .args(["-std=c11", "-fsyntax-only", "-I"])
                    .arg(dir.path())
                    .arg("-I")
                    .arg(root.join("include"))
                    .arg(dir.path().join("demo.c"))
                    .output()
                    .unwrap();
                assert!(!mismatch.status.success());
                assert!(
                    String::from_utf8_lossy(&mismatch.stderr)
                        .contains("Codec source/header profile mismatch")
                );
                fs::write(dir.path().join("demo_values.h"), old.values_header).unwrap();
                let out = Command::new("cc")
                    .args(["-std=c11", "-fsyntax-only", "-I"])
                    .arg(dir.path())
                    .arg("-I")
                    .arg(root.join("include"))
                    .arg("-I")
                    .arg(root.join("runtime/storage/include"))
                    .arg("-I")
                    .arg(root.join("tests/support"))
                    .arg("-I")
                    .arg(root.join("adapters/loopback/include"))
                    .arg(dir.path().join("test.c"))
                    .output()
                    .unwrap();
                assert!(!out.status.success());
                assert!(
                    String::from_utf8_lossy(&out.stderr).contains("Route codec/profile mismatch")
                );
            }
        }
    }
}
