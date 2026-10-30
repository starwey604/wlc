// SPDX-License-Identifier: Apache-2.0
//! Compact, reviewed artifact snapshots. Not a wire-format compatibility test.
use std::{fs, path::PathBuf};
use wlc::{
    ManifestArtifact, analyze_schema, binding_profile_identity, generate_c,
    generate_codegen_manifest, parse_binding_profile, parse_schema,
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
            Some(
                "profile version 2; endpoint { envelope=native_packet; } send Empty { delivery=unreliable; }",
            ),
        ),
        (
            "managed",
            include_str!("fixtures/codegen/runtime.wl").into(),
            Some(
                "profile version 2; endpoint { envelope=native_packet; } rpc Apply { request=Request; response=Response; } rpc Query { request=QueryRequest; response=QueryResponse; } send Telemetry { delivery=unreliable; } direct Event { delivery=unreliable; }",
            ),
        ),
    ];
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codegen");
    for (case, source, profile_source) in cases {
        let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let (artifacts, identity) = if let Some(source) = profile_source {
            let p = wlc::analyze_kernel_profile(&parse_binding_profile(source).unwrap(), &schema)
                .unwrap();
            let files = wlc::generate_kernel_c(&schema, &p, "business")
                .unwrap()
                .files;
            (
                files.into_iter().collect::<Vec<_>>(),
                Some(binding_profile_identity(p.bindings())),
            )
        } else {
            let c = generate_c(&schema, "business").unwrap();
            (
                vec![
                    ("business.h".into(), c.header),
                    ("business_values.h".into(), c.values_header),
                    ("business.c".into(), c.source),
                ],
                None,
            )
        };
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
