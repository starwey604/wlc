use wlc::{
    ARTIFACT_DIGEST_ALGORITHM, CODEGEN_ABI_VERSION, CODEGEN_MANIFEST_FORMAT, COMPILER_VERSION,
    ManifestArtifact, generate_codegen_manifest,
};

#[test]
fn manifest_is_order_independent_and_records_exact_provenance() {
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
        0x0123_4567_89ab_cdef,
        Some(0xfedc_ba98_7654_3210),
        &[header, source],
    );
    let second = generate_codegen_manifest(
        "control",
        0x0123_4567_89ab_cdef,
        Some(0xfedc_ba98_7654_3210),
        &[source, header],
    );

    assert_eq!(first, second);
    assert!(first.starts_with(&format!("{{\n  \"format\": \"{CODEGEN_MANIFEST_FORMAT}\"")));
    assert!(first.contains(&format!("\"version\": \"{COMPILER_VERSION}\"")));
    assert!(first.contains(&format!("\"codegen_abi\": {CODEGEN_ABI_VERSION}")));
    assert!(first.contains("\"schema\": \"0x0123456789abcdef\""));
    assert!(first.contains("\"binding_profile\": \"0xfedcba9876543210\""));
    assert!(first.contains(&format!(
        "\"artifact_digest_algorithm\": \"{ARTIFACT_DIGEST_ALGORITHM}\""
    )));
    assert!(first.find("control.c").unwrap() < first.find("control.h").unwrap());
}

#[test]
fn manifest_escapes_json_and_distinguishes_artifact_bytes() {
    let first = generate_codegen_manifest(
        "quote\"line\n",
        1,
        None,
        &[ManifestArtifact {
            path: "a\\b\t.c",
            contents: b"one",
        }],
    );
    let changed = generate_codegen_manifest(
        "quote\"line\n",
        1,
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
