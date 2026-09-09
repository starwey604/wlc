// SPDX-License-Identifier: Apache-2.0
//! Compact, reviewed artifact snapshots. Not a wire-format compatibility test.
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    ManifestArtifact, analyze_binding_profile, analyze_schema, binding_profile_identity,
    generate_c, generate_codegen_manifest, generate_runtime_c_named, parse_binding_profile,
    parse_schema,
};

fn codec_matrix() -> String {
    let mut schema = "version 1;\n".to_owned();
    let mut id = 1;
    for count in [0, 1, 8, 9, 16, 32, 64] {
        for stride in [1, 997] {
            schema.push_str(&format!("message N{count}S{stride} @id({id}) {{\n"));
            id += 1;
            for i in (0..count).rev() {
                schema.push_str(&format!("optional uint32 f{i} @id({});\n", 1 + i * stride));
            }
            schema.push_str("}\n");
        }
    }
    schema.push_str(include_str!("fixtures/codegen/scalars.wl.inc"));
    schema
}

#[test]
fn generated_artifacts_match_reviewed_snapshots() {
    let cases = [
        ("codec_matrix", codec_matrix(), None),
        (
            "empty",
            "version 1; message Empty @id(1) {}".into(),
            Some("profile version 1; send Empty { delivery = unreliable; }"),
        ),
        (
            "routes",
            include_str!("fixtures/codegen/runtime.wl").into(),
            Some(
                "profile version 1; latest Telemetry { delivery = unreliable; } fifo Event { delivery = reliable; } send Telemetry { delivery = reliable; } send Request { delivery = unreliable; }",
            ),
        ),
        (
            "managed",
            include_str!("fixtures/codegen/runtime.wl").into(),
            Some(
                "profile version 1; rpc Apply { request = Request; response = Response; } rpc Query { request = QueryRequest @delivery(unreliable); response = QueryResponse @delivery(unreliable); } latest Telemetry { delivery = unreliable; }",
            ),
        ),
        (
            "explicit",
            include_str!("fixtures/codegen/runtime.wl").into(),
            Some(
                "profile version 1; rpc Legacy { request = LegacyRequest; response = LegacyResponse; request_operation_id = operation_id; response_operation_id = call_id; response_status = status; request_delivery = reliable; response_delivery = unreliable; }",
            ),
        ),
    ];
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codegen");
    for (case, source, profile_source) in cases {
        let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let codec = generate_c(&schema, "business").unwrap();
        let mut artifacts = vec![
            ("business.h", codec.header),
            ("business_values.h", codec.values_header),
            ("business.c", codec.source),
            ("business_bindings.h", codec.bindings_header),
            ("business_bindings.c", codec.bindings_source),
        ];
        let identity = profile_source.map(|source| {
            let profile =
                analyze_binding_profile(&parse_binding_profile(source).unwrap(), &schema).unwrap();
            // A separate namespace must not rewrite codec references.
            let runtime =
                generate_runtime_c_named(&schema, &profile, "business", "station").unwrap();
            artifacts.extend([
                ("station_runtime.h", runtime.header),
                ("station_runtime.c", runtime.source),
                ("station_endpoint.h", runtime.endpoint_header),
                ("station_advanced.h", runtime.advanced_header),
            ]);
            binding_profile_identity(&profile)
        });
        // Optional exact-byte oracle for refactors; never required by ordinary CI.
        if let Some(compiler) = std::env::var_os("WLC_REFERENCE_COMPILER") {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("business.wl");
            fs::write(&input, &source).unwrap();
            let output = dir.path().join("out");
            let status = Command::new(&compiler)
                .arg("compile")
                .arg(&input)
                .arg("--out-dir")
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                status.status.success(),
                "{}",
                String::from_utf8_lossy(&status.stderr)
            );
            if let Some(source) = profile_source {
                let profile = dir.path().join("station.bind.wl");
                fs::write(&profile, source).unwrap();
                let status = Command::new(&compiler)
                    .arg("compile-runtime")
                    .arg(&input)
                    .arg("--profile")
                    .arg(profile)
                    .arg("--runtime-name")
                    .arg("station")
                    .arg("--out-dir")
                    .arg(&output)
                    .output()
                    .unwrap();
                assert!(
                    status.status.success(),
                    "{}",
                    String::from_utf8_lossy(&status.stderr)
                );
            }
            for (name, actual) in &artifacts {
                let expected = fs::read(output.join(name)).unwrap();
                let actual = actual.as_bytes();
                if expected != actual {
                    // Keep a failed refactor review readable: do not print a
                    // multi-megabyte debug array for an entire generated file.
                    let offset = expected
                        .iter()
                        .zip(actual)
                        .position(|(a, b)| a != b)
                        .unwrap_or(expected.len().min(actual.len()));
                    panic!(
                        "{case}/{name} differs from reference at byte {offset} (reference {} bytes, generated {} bytes)",
                        expected.len(),
                        actual.len()
                    );
                }
            }
        }
        let entries = artifacts
            .iter()
            .map(|(path, contents)| ManifestArtifact {
                path,
                contents: contents.as_bytes(),
            })
            .collect::<Vec<_>>();
        let actual = generate_codegen_manifest("business", &schema, identity, &entries);
        let path = golden.join(format!("{case}.json"));
        // Intentional API/output changes require explicit regeneration and review.
        if std::env::var_os("WLC_UPDATE_CODEGEN_SNAPSHOTS").is_some() {
            fs::write(&path, &actual).unwrap();
        }
        assert_eq!(fs::read_to_string(path).unwrap(), actual, "snapshot {case}");
    }
}
