use wlc::{
    analyze_schema,
    ast::{Cardinality, Declaration, Literal},
    check_compatibility, parse_schema,
    semantic::Symbol,
};

const VALID_SCHEMA: &str = r#"
// A complete, valid schema.
version 1;

enum State = 1 {
  STATE_UNKNOWN = 0;
  STATE_READY = 1;
}

message Status = 2 {
  optional State state = 1 [default = 0];
  optional string label = 2 [default = "ready"];
  repeated uint32 samples = 3;
}
"#;

#[test]
fn parses_all_frontend_constructs() {
    let schema = parse_schema(VALID_SCHEMA).expect("valid schema");
    assert_eq!(schema.version.value, 1);
    assert_eq!(schema.declarations.len(), 2);
    let Declaration::Message(message) = &schema.declarations[1] else {
        panic!("expected message");
    };
    assert_eq!(message.name.value, "Status");
    assert_eq!(message.fields[0].cardinality, Cardinality::Optional);
    assert_eq!(message.fields[2].cardinality, Cardinality::Repeated);
    assert_eq!(
        message.fields[1].default.as_ref().map(|value| &value.value),
        Some(&Literal::String("ready".to_owned()))
    );
}

#[test]
fn reports_precise_location_for_invalid_schema() {
    let error = parse_schema("version 1;\nmessage Ping = 1 {\n  optional bool ready = 0;\n}\n")
        .unwrap_err();
    assert_eq!(error.span.line, 3);
    assert_eq!(error.span.column, 25);
    assert_eq!(error.message, "field number must be in 1..=65535");
}

#[test]
fn rejects_repeated_default_values() {
    let error =
        parse_schema("version 1; message Batch = 1 { repeated uint32 ids = 1 [default = 0]; }")
            .unwrap_err();
    assert_eq!(error.span.line, 1);
    assert!(error.message.contains("repeated fields cannot"));
}

#[test]
fn rejects_duplicate_declaration_ids() {
    let error = parse_schema("version 1; enum One = 1 { A = 0; } message Two = 1 {} ").unwrap_err();
    assert_eq!(error.span.line, 1);
    assert!(error.message.contains("duplicate declaration id 1"));
}

#[test]
fn reserves_ids_at_the_correct_scope() {
    let schema = parse_schema(
        "version 1; reserved 7; message Packet = 2 { reserved 9; optional uint32 id = 1; }",
    )
    .expect("schema with reservations");
    assert_eq!(schema.reserved_ids[0].value, 7);
    let Declaration::Message(message) = &schema.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(message.reserved_numbers[0].value, 9);
}

#[test]
fn semantic_model_is_stable_across_declaration_and_field_order() {
    let first = parse_schema(
        "version 1; enum State = 2 { OFF = 0; ON = 1; } message Status = 1 { optional State state = 2 [default = 0]; optional uint32 sequence = 1; }",
    )
    .unwrap();
    let second = parse_schema(
        "version 1; message Status = 1 { optional uint32 sequence = 1; optional State state = 2 [default = 0]; } enum State = 2 { ON = 1; OFF = 0; }",
    )
    .unwrap();
    let first_model = analyze_schema(&first).unwrap();
    let second_model = analyze_schema(&second).unwrap();
    assert_eq!(
        first_model
            .declarations
            .iter()
            .map(Symbol::id)
            .collect::<Vec<_>>(),
        second_model
            .declarations
            .iter()
            .map(Symbol::id)
            .collect::<Vec<_>>()
    );
    let Symbol::Message(message) = &first_model.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(
        message
            .fields
            .iter()
            .map(|field| field.number)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn semantic_analysis_rejects_unknown_types_and_invalid_enum_defaults() {
    let unknown =
        parse_schema("version 1; message Status = 1 { optional Missing value = 1; }").unwrap();
    let errors = analyze_schema(&unknown).unwrap_err();
    assert!(
        errors.errors()[0]
            .message
            .contains("unknown field type `Missing`")
    );

    let invalid_default = parse_schema(
        "version 1; enum State = 1 { OFF = 0; } message Status = 2 { optional State state = 1 [default = 3]; }",
    )
    .unwrap();
    let errors = analyze_schema(&invalid_default).unwrap_err();
    assert!(errors.errors()[0].message.contains("default value 3"));
}

#[test]
fn compatibility_requires_removed_ids_to_remain_reserved() {
    let previous = analyze_schema(
        &parse_schema("version 1; message Status = 1 { optional uint32 sequence = 1; optional bool ready = 2; }").unwrap(),
    )
    .unwrap();
    let compatible = analyze_schema(
        &parse_schema(
            "version 2; message Status = 1 { optional uint32 sequence = 1; reserved 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    check_compatibility(&previous, &compatible).expect("removed field is reserved");

    let incompatible = analyze_schema(
        &parse_schema("version 2; message Status = 1 { optional uint32 sequence = 1; }").unwrap(),
    )
    .unwrap();
    let errors = check_compatibility(&previous, &incompatible).unwrap_err();
    assert!(
        errors.errors()[0]
            .message
            .contains("must be retained as `reserved 2;`")
    );
}

#[test]
fn compatibility_rejects_reused_declaration_ids_and_changed_field_numbers() {
    let previous = analyze_schema(
        &parse_schema("version 1; message Status = 1 { optional uint32 sequence = 1; }").unwrap(),
    )
    .unwrap();
    let current = analyze_schema(
        &parse_schema(
            "version 2; message Status = 1 { reserved 1; optional uint32 sequence = 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    let errors = check_compatibility(&previous, &current).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("changed number from 1 to 2"))
    );

    let reused =
        analyze_schema(&parse_schema("version 2; enum Status = 1 { OFF = 0; }").unwrap()).unwrap();
    let errors = check_compatibility(&previous, &reused).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("changed kind"))
    );
}
