use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn compact_managed_wire_preserves_full_domains_and_tagged_evolution() {
    let schema = analyze_schema(&parse_schema(r#"
version 1;
message Child @id(1) { optional fixed32 scalar @id(1) [default = 4294967295]; required packed fixed32 array[2] @id(2); }
message Request @id(2) { required fixed32 first @id(1); required fixed32 second @id(2); required fixed32 third @id(3); }
message Response @id(3) { optional fixed32 reply @id(1); optional Child nested @id(2); }
message WlcFoo @id(4) { required fixed32 wlc_foo_encode @id(1); }
message RequestExtra @id(5) { optional fixed32 request_extra_encode @id(1); optional fixed32 wlc_clear @id(2); }
message Telemetry @id(6) { required string<32> label @id(1); }
"#).unwrap()).unwrap();
    let profile = analyze_binding_profile(&parse_binding_profile("profile version 1; rpc Execute { request=Request @delivery(unreliable); response=Response @delivery(unreliable); } rpc Names { request=WlcFoo @delivery(unreliable); response=RequestExtra @delivery(unreliable); } direct Telemetry { delivery=unreliable; }").unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "wire").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "wire").unwrap();
    assert!(
        runtime
            .header
            .contains("RPC_MANAGED_WIRE_FORMAT \"managed-rpc-v4-tagged-varuint32\"")
    );
    // Public fixed32 payload maximum stays 3 * (one-byte key + four-byte value).
    assert!(
        codec
            .values_header
            .contains("REQUEST_MAX_ENCODED_SIZE UINT64_C(15)")
    );
    // Private varuint32 payload needs up to 18 bytes, plus 13 bytes of metadata.
    assert!(
        runtime
            .header
            .contains("WIRE_ENDPOINT_REQUEST_CAPACITY 31U")
    );
    assert!(!runtime.header.contains("client_decode"));
    assert!(!runtime.source.contains("decode_owned_response"));
    // Keep nested RPC dependencies and identifier-collision coverage, but do not
    // clone an ordinary route's codec or hash a private wire representation.
    assert!(runtime.source.contains("wire_rpc_child_decode"));
    assert!(!runtime.source.contains("wire_rpc_telemetry_"));
    assert!(!runtime.source.contains("wire_rpc_wlc_utf8_valid"));
    assert!(!runtime.source.contains("wire_rpc_wlc_hash_"));
    assert!(
        runtime
            .source
            .contains("wire_rpc_request_wlc_detail_fingerprint")
    );
    assert!(
        !runtime
            .source
            .contains("wire_rpc_response_wlc_detail_fingerprint")
    );
    let signature = "static inline wl_codec_status_t wire_rpc_response_wlc_detail_value_decode(const uint8_t *input, size_t length, response_value_t *out) {";
    assert_eq!(runtime.source.matches(signature).count(), 1);
    let counted = format!(
        "static unsigned response_decode_count;\n{}",
        runtime.source.replace(
            signature,
            &format!("{signature}\n  ++response_decode_count;")
        )
    );
    let dir = tempdir().unwrap();
    for (name, text) in [
        ("wire.h", codec.header),
        ("wire_values.h", codec.values_header),
        ("wire.c", codec.source),
        ("wire_bindings.h", codec.bindings_header),
        ("wire_bindings.c", codec.bindings_source),
        ("wire_runtime.h", runtime.header),
        ("wire_runtime.c", counted),
        (
            "test.c",
            include_str!("fixtures/managed_rpc_wire.c").to_owned(),
        ),
    ] {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut sources = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect::<Vec<_>>();
    sources.sort();
    let binary = dir.path().join("test");
    let mut command = Command::new("cc");
    command
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(dir.path())
        .arg(dir.path().join("test.c"))
        .arg(dir.path().join("wire.c"))
        .arg(dir.path().join("wire_bindings.c"))
        .args(sources)
        .arg("-o")
        .arg(&binary);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        command.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let result = command.output().unwrap();
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
}
