use wlc::{
    ast::{Cardinality, Declaration, Literal},
    parse_schema,
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
