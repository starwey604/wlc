use proptest::prelude::*;
use wlc::{
    ManifestArtifact, analyze_binding_profile, analyze_schema,
    ast::{Cardinality, Declaration, Literal},
    binding_profile_identity, check_compatibility, generate_c, generate_codegen_manifest,
    generate_runtime_c, parse_binding_profile, parse_schema, schema_identity,
};

const ATTRIBUTES: &str = r#"
version 1;
reserved 99;
enum State @id(1) {
  UNKNOWN = 0;
  READY = 1;
  reserved -1;
}
message Child @id(2) {
  required uint32 sample @id(1);
}
message Telemetry @id(10) {
  reserved 10;
  optional State state @id(1) [default = 0];
  optional Child child @id(2);
  packed float32 joints[6] @id(3);
  required packed fixed32 ticks[2] @id(4);
  required int32 temperature_centi_c @id(5);
}
message Metadata @id(11) {
  optional string<31> label @id(1) [default = "ready"];
  required bytes<255> payload @id(2);
  repeated uint32 samples @id(3);
}
"#;

const LEGACY: &str = r#"
version 1;
reserved 99;
enum State = 1 {
  UNKNOWN = 0;
  READY = 1;
  reserved -1;
}
message Child = 2 {
  required uint32 sample = 1;
}
message Telemetry = 10 {
  reserved 10;
  optional State state = 1 [default = 0];
  optional Child child = 2;
  packed float32 joints[6] = 3;
  required packed fixed32 ticks[2] = 4;
  required int32 temperature_centi_c = 5;
}
message Metadata = 11 {
  optional string<31> label = 1 [default = "ready"];
  required bytes<255> payload = 2;
  repeated uint32 samples = 3;
}
"#;

#[test]
fn attributes_cover_declarations_cardinalities_bounds_and_defaults() {
    let schema = parse_schema(ATTRIBUTES).unwrap();
    let Declaration::Enum(state) = &schema.declarations[0] else {
        panic!("expected enum");
    };
    assert_eq!(state.id.value, 1);
    assert_eq!(state.values[0].number.value, 0);
    assert_eq!(state.reserved_numbers[0].value, -1);
    let Declaration::Message(telemetry) = &schema.declarations[2] else {
        panic!("expected telemetry message");
    };
    assert_eq!(telemetry.id.value, 10);
    assert_eq!(telemetry.fields[0].number.value, 1);
    assert_eq!(telemetry.fields[2].cardinality, Cardinality::Packed(6));
    assert_eq!(
        telemetry.fields[3].cardinality,
        Cardinality::RequiredPacked(2)
    );
    assert_eq!(telemetry.fields[4].cardinality, Cardinality::Required);
    let Declaration::Message(metadata) = &schema.declarations[3] else {
        panic!("expected metadata message");
    };
    assert_eq!(metadata.fields[0].max_length.as_ref().unwrap().value, 31);
    assert_eq!(
        metadata.fields[0].default.as_ref().unwrap().value,
        Literal::String("ready".to_owned())
    );
    assert_eq!(metadata.fields[2].cardinality, Cardinality::Repeated);
    analyze_schema(&schema).unwrap();
}

#[test]
fn attributes_and_legacy_syntax_have_identical_generated_artifacts_and_identities() {
    let legacy = analyze_schema(&parse_schema(LEGACY).unwrap()).unwrap();
    let attributes = analyze_schema(&parse_schema(ATTRIBUTES).unwrap()).unwrap();
    assert_eq!(schema_identity(&legacy), schema_identity(&attributes));
    // Compatibility checks describe a revision upgrade, not a spelling-only
    // rewrite. Both spellings permit the same otherwise-identical next revision.
    let mut next_legacy = legacy.clone();
    let mut next_attributes = attributes.clone();
    next_legacy.version += 1;
    next_attributes.version += 1;
    check_compatibility(&legacy, &next_attributes).unwrap();
    check_compatibility(&attributes, &next_legacy).unwrap();

    let profile =
        parse_binding_profile("profile version 1; latest Telemetry { delivery = unreliable; }")
            .unwrap();
    let mut outputs = Vec::new();
    for schema in [&legacy, &attributes] {
        let profile = analyze_binding_profile(&profile, schema).unwrap();
        let identity = binding_profile_identity(&profile);
        let codec = generate_c(schema, "id_syntax").unwrap();
        let runtime = generate_runtime_c(schema, &profile, "id_syntax").unwrap();
        let manifest = generate_codegen_manifest(
            "id_syntax",
            schema,
            Some(identity),
            &[
                ManifestArtifact {
                    path: "id_syntax.h",
                    contents: codec.header.as_bytes(),
                },
                ManifestArtifact {
                    path: "id_syntax.c",
                    contents: codec.source.as_bytes(),
                },
                ManifestArtifact {
                    path: "id_syntax_bindings.h",
                    contents: codec.bindings_header.as_bytes(),
                },
                ManifestArtifact {
                    path: "id_syntax_bindings.c",
                    contents: codec.bindings_source.as_bytes(),
                },
                ManifestArtifact {
                    path: "id_syntax_runtime.h",
                    contents: runtime.header.as_bytes(),
                },
                ManifestArtifact {
                    path: "id_syntax_runtime.c",
                    contents: runtime.source.as_bytes(),
                },
            ],
        );
        outputs.push((identity, codec, runtime, manifest));
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn legacy_and_attribute_spelling_can_mix_without_reserving_id_as_a_keyword() {
    let schema = parse_schema(
        "version 1; enum id @id(65535) { id = 0; } \
         message Packet = 1 { optional id id @id(65535); required uint32 value = 2; }",
    )
    .unwrap();
    analyze_schema(&schema).unwrap();
    let Declaration::Message(message) = &schema.declarations[1] else {
        panic!("expected message");
    };
    assert_eq!(message.fields[0].name.value, "id");
    assert_eq!(message.fields[0].number.value, 65535);
    assert_eq!(message.fields[1].number.value, 2);
}

#[test]
fn attributes_allow_ordinary_whitespace_and_comments() {
    let source = "version 1; message Packet @ // attribute\n id ( // number\n 1 ) { \
                  optional uint32 value @ id ( 2 ); }";
    let schema = parse_schema(source).unwrap();
    analyze_schema(&schema).unwrap();
}

#[test]
fn malformed_attributes_are_rejected_with_actionable_diagnostics() {
    for (source, expected) in [
        (
            "version 1; message Packet @tag(1) {}",
            "unknown schema attribute `@tag`",
        ),
        (
            "version 1; message Packet @ {}",
            "expected `id` attribute name",
        ),
        (
            "version 1; message Packet @id 1 {}",
            "expected `(` after `@id`",
        ),
        ("version 1; message Packet @id() {}", "expected message id"),
        (
            "version 1; message Packet @id(1 {}",
            "expected `)` after ID",
        ),
        (
            "version 1; message Packet @id(1 2) {}",
            "expected `)` after ID",
        ),
        ("version 1; message Packet @id(1) @id(2) {}", "expected `{`"),
        ("version 1; message Packet @id(1) = 2 {}", "expected `{`"),
        ("version 1; message Packet = 1 @id(2) {}", "expected `{`"),
        (
            "version 1; message Packet {}",
            "expected `@id(...)` or legacy `= number`",
        ),
        (
            "version 1; message Packet @id(1) { optional uint32 value @id(1) @id(2); }",
            "expected `;`",
        ),
        (
            "version 1; message Packet @id(1) { optional uint32 value @tag(1); }",
            "unknown schema attribute `@tag`",
        ),
        (
            "version 1; enum State @id(1) { OK @id(0); }",
            "expected `=` after enum value name",
        ),
    ] {
        let error = parse_schema(source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn annotated_ids_reuse_nonzero_u16_validation_and_numeric_error_spans() {
    for invalid in ["0", "-1", "65536", "4294967296"] {
        for (source, kind) in [
            (
                format!("version 1; message Packet @id({invalid}) {{}}"),
                "message id",
            ),
            (
                format!("version 1; enum State @id({invalid}) {{ OK = 0; }}"),
                "enum id",
            ),
            (
                format!(
                    "version 1; message Packet @id(1) {{ optional uint32 value @id({invalid}); }}"
                ),
                "field number",
            ),
        ] {
            let error = parse_schema(&source).unwrap_err();
            assert_eq!(error.message, format!("{kind} must be in 1..=65535"));
            assert_eq!(
                &source[error.span.offset..error.span.offset + error.span.length],
                invalid
            );
        }
    }
}

#[test]
fn annotations_preserve_duplicate_and_reserved_id_checks() {
    for (source, expected) in [
        (
            "version 1; message A @id(1) {} enum B @id(1) { OK = 0; }",
            "duplicate declaration id 1",
        ),
        (
            "version 1; message A = 1 {} message B @id(1) {}",
            "duplicate declaration id 1",
        ),
        (
            "version 1; message A @id(1) { optional uint32 a @id(1); optional uint32 b = 1; }",
            "duplicate field number 1",
        ),
        (
            "version 1; reserved 1; message A @id(1) {}",
            "declaration ID 1 is both active and reserved",
        ),
        (
            "version 1; enum A @id(1) { OK = 0; } reserved 1;",
            "declaration ID 1 is both active and reserved",
        ),
        (
            "version 1; message A @id(1) { reserved 2; optional uint32 a @id(2); }",
            "field number 2 is both active and reserved",
        ),
        (
            "version 1; message A @id(1) { optional uint32 a @id(2); reserved 2; }",
            "field number 2 is both active and reserved",
        ),
    ] {
        let error = parse_schema(source).unwrap_err();
        assert_eq!(error.message, expected, "{source}");
    }
}

#[test]
fn annotations_do_not_relax_default_value_rules() {
    for cardinality in ["required", "repeated", "packed"] {
        let name = if cardinality == "packed" {
            "values[2]"
        } else {
            "value"
        };
        let source = format!(
            "version 1; message Packet @id(1) {{ {cardinality} fixed32 {name} @id(1) [default = 1]; }}"
        );
        let error = parse_schema(&source).unwrap_err();
        assert_eq!(
            error.message,
            format!("{cardinality} fields cannot declare a default value")
        );
    }
}

proptest! {
    #[test]
    fn equivalent_spelling_preserves_identity_for_all_valid_id_ranges(
        message_id in 1_u16..=u16::MAX,
        field_id in 1_u16..=u16::MAX,
    ) {
        let source = format!("version 1; message Packet @id({message_id}) {{ required uint32 sample @id({field_id}); }}");
        let legacy = format!("version 1; message Packet = {message_id} {{ required uint32 sample = {field_id}; }}");
        let source = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let legacy = analyze_schema(&parse_schema(&legacy).unwrap()).unwrap();
        prop_assert_eq!(schema_identity(&source), schema_identity(&legacy));
        prop_assert_eq!(generate_c(&source, "packet").unwrap(), generate_c(&legacy, "packet").unwrap());
    }
}
