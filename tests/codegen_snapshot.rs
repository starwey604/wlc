// SPDX-License-Identifier: Apache-2.0
//! Compact, reviewed artifact snapshots. Not a wire-format compatibility test.
use std::{fs, path::PathBuf};
use wlc::{
    ManifestArtifact, analyze_schema, generate_c, generate_codegen_manifest, node_identity,
    parse_schema,
};
use wlc::{node::analyze_nodes, node_codegen::generate_node_c_with_pruning};

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
            Some("node empty { send Empty; }"),
        ),
        (
            "managed",
            include_str!("fixtures/codegen/runtime.wl").into(),
            Some(
                "rpc Apply(Request) -> Response; rpc Query(QueryRequest) -> QueryResponse; node business { call Apply; serve Apply; call Query; serve Query; send Telemetry; recv Event; }",
            ),
        ),
    ];
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codegen");
    for (case, source, node_source) in cases {
        let source = format!("{source}\n{}", node_source.unwrap_or(""));
        let ast = parse_schema(&source).unwrap();
        let schema = analyze_schema(&ast).unwrap();
        let (artifacts, identity) = if node_source.is_some() {
            let nodes = analyze_nodes(&ast, &schema).unwrap();
            let files = generate_node_c_with_pruning(&schema, &nodes, "business", true)
                .unwrap()
                .files;
            (
                files.into_iter().collect::<Vec<_>>(),
                Some(node_identity(&nodes[0])),
            )
        } else {
            let c = generate_c(&schema, "business").unwrap();
            (
                vec![
                    ("business.h".into(), c.header),
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
