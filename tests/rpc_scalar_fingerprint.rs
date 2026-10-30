// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn local_scalar_fingerprint_matches_schema_v1_with_both_rpc_codecs() {
    let schema = analyze_schema(
        &parse_schema(
            "version 1;
        message Scalar @id(1) {
            required uint32 a @id(1);
            optional uint32 b @id(16) [default = 4294967295];
            required fixed32 c @id(2048);
            optional fixed64 d @id(2049);
            required float32 e @id(2050);
            optional float64 f @id(65534);
            optional fixed32 g @id(65535);
        }
        message Reply @id(2) { optional fixed32 value @id(1); }
        message Empty @id(3) { optional fixed64 value @id(1); }
        message Other @id(4) { optional uint32 value @id(1); }",
        )
        .unwrap(),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    for codec_name in ["managed_tagged", "positional_v1"] {
        let profile = analyze_binding_profile(&parse_binding_profile(&format!(
            "profile version 1; rpc Execute {{ request=Scalar @delivery(unreliable); response=Reply @delivery(unreliable); codec={codec_name}; }} rpc Optional {{ request=Empty; response=Other; codec={codec_name}; }}"
        )).unwrap(), &schema).unwrap();
        let codec = generate_c(&schema, "fp").unwrap();
        let runtime = generate_runtime_c(&schema, &profile, "fp").unwrap();
        let dir = tempdir().unwrap();
        for (name, contents) in [
            ("fp.h", codec.header),
            ("fp_values.h", codec.values_header),
            ("fp.c", codec.source),
            ("fp_bindings.h", codec.bindings_header),
            ("fp_bindings.c", codec.bindings_source),
            ("fp_runtime.h", runtime.header),
            ("fp_runtime.c", runtime.source),
            (
                "test.c",
                include_str!("fixtures/rpc_scalar_fingerprint.c").into(),
            ),
        ] {
            fs::write(dir.path().join(name), contents).unwrap();
        }
        let mut sources = fs::read_dir(root.join("src"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "c"))
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
            .arg(dir.path().join("fp.c"))
            .arg(dir.path().join("fp_bindings.c"))
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
            "{codec_name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
