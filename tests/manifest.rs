use wlc::{
    ARTIFACT_DIGEST_ALGORITHM, CODEGEN_ABI_VERSION, CODEGEN_MANIFEST_FORMAT, COMPILER_VERSION,
    ManifestArtifact, analyze_schema, generate_codegen_manifest, parse_schema, schema_identity,
};

fn schema(source: &str) -> wlc::SemanticModel {
    analyze_schema(&parse_schema(source).unwrap()).unwrap()
}

#[test]
fn manifest_is_order_independent_and_records_exact_provenance() {
    let schema = schema("version 1; message Control = 1 {}");
    let header = ManifestArtifact {
        path: "control.h",
        contents: b"header\n",
    };
    let source = ManifestArtifact {
        path: "control.c",
        contents: b"source\n",
    };
    let first = generate_codegen_manifest(
        "control",
        &schema,
        Some(0xfedc_ba98_7654_3210),
        &[header, source],
    );
    let second = generate_codegen_manifest(
        "control",
        &schema,
        Some(0xfedc_ba98_7654_3210),
        &[source, header],
    );

    assert_eq!(first, second);
    assert!(first.starts_with(&format!("{{\n  \"format\": \"{CODEGEN_MANIFEST_FORMAT}\"")));
    assert!(first.contains(&format!("\"version\": \"{COMPILER_VERSION}\"")));
    assert!(first.contains(&format!("\"codegen_abi\": {CODEGEN_ABI_VERSION}")));
    assert!(first.contains(&format!(
        "\"schema\": \"0x{:016x}\"",
        schema_identity(&schema)
    )));
    assert!(first.contains("\"binding_profile\": \"0xfedcba9876543210\""));
    assert!(first.contains(&format!(
        "\"artifact_digest_algorithm\": \"{ARTIFACT_DIGEST_ALGORITHM}\""
    )));
    assert!(first.find("control.c").unwrap() < first.find("control.h").unwrap());
}

#[test]
fn manifest_escapes_json_and_distinguishes_artifact_bytes() {
    let schema = schema("version 1; message Control = 1 {}");
    let first = generate_codegen_manifest(
        "quote\"line\n",
        &schema,
        None,
        &[ManifestArtifact {
            path: "a\\b\t.c",
            contents: b"one",
        }],
    );
    let changed = generate_codegen_manifest(
        "quote\"line\n",
        &schema,
        None,
        &[ManifestArtifact {
            path: "a\\b\t.c",
            contents: b"two",
        }],
    );

    assert!(first.contains("\"module\": \"quote\\\"line\\n\""));
    assert!(first.contains("\"path\": \"a\\\\b\\t.c\""));
    assert!(first.contains("\"binding_profile\": null"));
    assert_ne!(first, changed);
}

#[test]
fn manifest_records_bounded_string_and_bytes_fields() {
    let schema = schema(
        r#"version 1;
message Metadata = 7 {
  optional string<31> name = 2;
  repeated bytes<255> chunks = 3;
  optional string unbounded = 4;
}
"#,
    );
    let manifest = generate_codegen_manifest("metadata", &schema, None, &[]);

    assert!(manifest.contains(
        r#"{"message": "Metadata", "message_id": 7, "field": "name", "field_number": 2, "kind": "string", "max_length": 31}"#
    ));
    assert!(manifest.contains(
        r#"{"message": "Metadata", "message_id": 7, "field": "chunks", "field_number": 3, "kind": "bytes", "max_length": 255}"#
    ));
    assert!(!manifest.contains("unbounded\""));
}
