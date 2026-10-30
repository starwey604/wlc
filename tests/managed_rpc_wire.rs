// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_kernel_profile, analyze_schema, generate_kernel_c, parse_binding_profile, parse_schema,
};
#[test]
fn compact_managed_wire_preserves_frozen_public_and_private_domains() {
    let schema = analyze_schema(&parse_schema(r#"
version 1;
message Child @id(1) { optional fixed32 scalar @id(1) [default = 4294967295]; required packed fixed32 array[2] @id(2); }
message Request @id(2) { required fixed32 first @id(1); required fixed32 second @id(2); required fixed32 third @id(3); }
message Response @id(3) { optional fixed32 reply @id(1); optional Child nested @id(2); }
message WlcFoo @id(4) { required fixed32 wlc_foo_encode @id(1); }
message RequestExtra @id(5) { optional fixed32 request_extra_encode @id(1); optional fixed32 wlc_clear @id(2); }
message Telemetry @id(6) { required string<32> label @id(1); }
"#).unwrap()).unwrap();

    let profile=analyze_kernel_profile(&parse_binding_profile("profile version 2; endpoint {envelope=native_packet;} rpc Execute {request=Request;response=Response;} rpc Names {request=WlcFoo;response=RequestExtra;} direct Telemetry {delivery=unreliable;}").unwrap(),&schema).unwrap();
    let files = generate_kernel_c(&schema, &profile, "wire").unwrap().files;
    assert!(files["wire_values.h"].contains("REQUEST_MAX_ENCODED_SIZE UINT64_C(15)"));
    assert!(files["wire_kernel.h"].contains("WIRE_KERNEL_REQUEST_FRAME_MAX 35U"));
    assert!(!files["wire_kernel_codecs.h"].contains("fingerprint"));
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in files {
        fs::write(dir.path().join(name), contents).unwrap();
    }
    fs::write(
        dir.path().join("test.c"),
        include_str!("fixtures/managed_rpc_wire.c"),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut cc = Command::new(std::env::var("CC").unwrap_or("cc".into()));
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]);
    }
    let result = cc
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("wire.c"))
        .arg(root.join("src/rpc_wire.c"))
        .arg(dir.path().join("test.c"))
        .arg("-o")
        .arg(dir.path().join("test"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        Command::new(dir.path().join("test"))
            .status()
            .unwrap()
            .success()
    );
}
