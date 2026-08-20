use wlc::{
    analyze_schema,
    ast::{Cardinality, Declaration, Literal},
    check_compatibility, generate_c, parse_schema,
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

#[test]
fn semantic_analysis_accepts_v1_builtins_and_full_width_defaults() {
    let schema = parse_schema(
        r#"version 1;
        enum Mode = 1 { OFF = -2147483648; ON = 2147483647; }
        message Scalars = 2 {
          optional bool enabled = 1 [default = false];
          optional uint64 maximum = 2 [default = 18446744073709551615];
          optional int64 minimum = 3 [default = -9223372036854775808];
          optional fixed32 mask = 4 [default = 4294967295];
          optional fixed64 wide_mask = 5 [default = 18446744073709551615];
          optional string label = 6 [default = "电机"];
          optional Mode mode = 7 [default = -2147483648];
          optional bytes payload = 8;
        }"#,
    )
    .unwrap();
    analyze_schema(&schema).expect("all v1 builtins should resolve");
}

#[test]
fn semantic_analysis_rejects_invalid_defaults_and_message_cycles() {
    let invalid_defaults = parse_schema(
        "version 1; message Value = 1 { optional uint32 u = 1 [default = -1]; optional bytes b = 2 [default = 0]; }",
    )
    .unwrap();
    let errors = analyze_schema(&invalid_defaults).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("uint32"))
    );
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("bytes fields"))
    );

    let cycle = parse_schema(
        "version 1; message A = 1 { optional B b = 1; } message B = 2 { optional A a = 1; }",
    )
    .unwrap();
    let errors = analyze_schema(&cycle).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("recursive"))
    );
}

#[test]
fn semantic_analysis_enforces_eight_nested_message_levels() {
    let mut source = String::from("version 1;");
    for id in 1..=10 {
        let next = id + 1;
        if id == 10 {
            source.push_str(&format!(" message M{id} = {id} {{}}"));
        } else {
            source.push_str(&format!(
                " message M{id} = {id} {{ optional M{next} child = 1; }}"
            ));
        }
    }
    let schema = parse_schema(&source).unwrap();
    let errors = analyze_schema(&schema).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("depth exceeds"))
    );
}

#[test]
fn compatibility_requires_a_strictly_new_revision() {
    let previous = analyze_schema(
        &parse_schema("version 2; message Status = 1 { optional bool ready = 1; }").unwrap(),
    )
    .unwrap();
    let current = analyze_schema(
        &parse_schema("version 2; message Status = 1 { optional bool ready = 1; }").unwrap(),
    )
    .unwrap();
    let errors = check_compatibility(&previous, &current).unwrap_err();
    assert!(errors.errors()[0].message.contains("must increase"));
}

#[test]
fn generates_deterministic_c_data_model_and_api() {
    let model = analyze_schema(&parse_schema(VALID_SCHEMA).unwrap()).unwrap();
    let generated = generate_c(&model, "motor_api").unwrap();
    assert!(generated.header.contains("typedef int32_t state_t;"));
    assert!(generated.header.contains("struct status {"));
    assert!(generated.header.contains("bool has_state;"));
    assert!(generated.header.contains("uint32_t *samples;"));
    assert!(generated.header.contains("wl_codec_status_t status_decode"));
    assert!(generated.source.contains("void status_clear"));
}
