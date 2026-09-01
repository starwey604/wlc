use wlc::{
    analyze_schema,
    ast::{Cardinality, Declaration, Literal},
    check_compatibility, generate_c, parse_schema,
    semantic::{FieldDefault, ResolvedType, Symbol},
};

use proptest::prelude::*;

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
fn parses_float_scalars_and_fixed_packed_arrays() {
    let schema = parse_schema(
        "version 1; message Control = 1 { optional float32 position = 1; optional float64 time = 2; packed float32 joints[6] = 3; packed fixed64 ticks[2] = 4; }",
    )
    .expect("dense numeric schema");
    let Declaration::Message(message) = &schema.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(message.fields[0].ty.value, "float32");
    assert_eq!(message.fields[1].ty.value, "float64");
    assert_eq!(message.fields[2].cardinality, Cardinality::Packed(6));
    assert_eq!(message.fields[3].cardinality, Cardinality::Packed(2));
    analyze_schema(&schema).expect("float and supported packed types resolve");
}

#[test]
fn parses_required_scalar_nested_and_packed_fields() {
    let schema = parse_schema(
        "version 1; message Child = 1 { required uint32 value = 1; } message Packet = 2 { required Child child = 1; required packed float32 joints[6] = 2; optional uint32 required = 3; }",
    )
    .expect("required fields");
    let Declaration::Message(packet) = &schema.declarations[1] else {
        panic!("expected packet message");
    };
    assert_eq!(packet.fields[0].cardinality, Cardinality::Required);
    assert_eq!(packet.fields[1].cardinality, Cardinality::RequiredPacked(6));
    assert_eq!(packet.fields[2].name.value, "required");
    analyze_schema(&schema).expect("required fields should resolve");
}

#[test]
fn parses_bounded_borrowed_fields_for_every_cardinality() {
    let schema = parse_schema(
        r#"version 1;
message Metadata = 1 {
  optional string<31> name = 1 [default = "ready"];
  required bytes<65535> payload = 2;
  repeated string<1> labels = 3;
}
"#,
    )
    .unwrap();
    let Declaration::Message(parsed) = &schema.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(parsed.fields[0].max_length.as_ref().unwrap().value, 31);
    assert_eq!(parsed.fields[1].max_length.as_ref().unwrap().value, 65535);
    assert_eq!(parsed.fields[2].max_length.as_ref().unwrap().value, 1);

    let model = analyze_schema(&schema).unwrap();
    let Symbol::Message(message) = &model.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(message.fields[0].ty, ResolvedType::String);
    assert_eq!(message.fields[0].max_length, Some(31));
    assert_eq!(message.fields[1].ty, ResolvedType::Bytes);
    assert_eq!(message.fields[1].max_length, Some(65535));
    assert_eq!(message.fields[2].max_length, Some(1));
}

#[test]
fn bounded_field_diagnostics_reject_invalid_ranges_types_and_defaults() {
    for source in [
        "version 1; message Bad = 1 { optional string<0> value = 1; }",
        "version 1; message Bad = 1 { optional bytes<65536> value = 1; }",
    ] {
        let error = parse_schema(source).unwrap_err();
        assert!(
            error
                .message
                .contains("string/bytes bound must be in 1..=65535")
        );
    }

    let wrong_type = analyze_schema(
        &parse_schema("version 1; message Bad = 1 { optional uint32<8> value = 1; }").unwrap(),
    )
    .unwrap_err();
    assert!(wrong_type.errors().iter().any(|error| {
        error
            .message
            .contains("may only declare a length bound on string or bytes")
    }));

    analyze_schema(
        &parse_schema(
            "version 1; message Good = 1 { optional string<3> value = 1 [default = \"电\"]; }",
        )
        .unwrap(),
    )
    .expect("the bound counts encoded UTF-8 bytes");
    let too_long = analyze_schema(
        &parse_schema(
            "version 1; message Bad = 1 { optional string<2> value = 1 [default = \"电\"]; }",
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(too_long.errors().iter().any(|error| {
        error
            .message
            .contains("default string length 3 exceeds declared bound 2 bytes")
    }));
}

#[test]
fn resolves_narrow_integer_types_and_range_checked_defaults() {
    let schema = parse_schema(
        r#"version 1;
message Narrow = 1 {
  optional uint8 unsigned_8 = 1 [default = 255];
  optional uint16 unsigned_16 = 2 [default = 65535];
  optional int8 signed_8 = 3 [default = -128];
  optional int16 signed_16 = 4 [default = -32768];
}
"#,
    )
    .unwrap();
    let Declaration::Message(parsed) = &schema.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(
        parsed
            .fields
            .iter()
            .map(|field| field.ty.value.as_str())
            .collect::<Vec<_>>(),
        ["uint8", "uint16", "int8", "int16"]
    );

    let model = analyze_schema(&schema).unwrap();
    let Symbol::Message(message) = &model.declarations[0] else {
        panic!("expected message");
    };
    assert_eq!(message.fields[0].ty, ResolvedType::Uint8);
    assert_eq!(message.fields[0].default, Some(FieldDefault::Uint8(255)));
    assert_eq!(message.fields[1].ty, ResolvedType::Uint16);
    assert_eq!(message.fields[1].default, Some(FieldDefault::Uint16(65535)));
    assert_eq!(message.fields[2].ty, ResolvedType::Int8);
    assert_eq!(message.fields[2].default, Some(FieldDefault::Int8(-128)));
    assert_eq!(message.fields[3].ty, ResolvedType::Int16);
    assert_eq!(message.fields[3].default, Some(FieldDefault::Int16(-32768)));

    for (ty, value) in [
        ("uint8", "-1"),
        ("uint8", "256"),
        ("uint16", "65536"),
        ("int8", "-129"),
        ("int8", "128"),
        ("int16", "-32769"),
        ("int16", "32768"),
    ] {
        let source = format!(
            "version 1; message Bad = 1 {{ optional {ty} value = 1 [default = {value}]; }}"
        );
        let errors = analyze_schema(&parse_schema(&source).unwrap()).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains(&format!("does not fit {ty}"))),
            "unexpected errors for {ty} default {value}: {:?}",
            errors.errors()
        );
    }
}

#[test]
fn rejects_required_repeated_and_required_defaults() {
    for (source, expected) in [
        (
            "version 1; message Bad = 1 { required repeated uint32 values = 1; }",
            "required fields cannot be repeated",
        ),
        (
            "version 1; message Bad = 1 { required uint32 value = 1 [default = 0]; }",
            "required fields cannot declare a default value",
        ),
        (
            "version 1; message Bad = 1 { required packed fixed32 values[2] = 1 [default = 0]; }",
            "required fields cannot declare a default value",
        ),
    ] {
        let error = parse_schema(source).unwrap_err();
        assert!(
            error.message.contains(expected),
            "unexpected diagnostic: {error}"
        );
    }
}

#[test]
fn packed_arrays_require_a_valid_count_and_fixed_width_numeric_type() {
    for source in [
        "version 1; message Bad = 1 { packed float32 values[0] = 1; }",
        "version 1; message Bad = 1 { packed float32 values[65536] = 1; }",
    ] {
        let error = parse_schema(source).unwrap_err();
        assert!(error.message.contains("packed element count"));
    }

    let schema = parse_schema(
        "version 1; message Bad = 1 { packed uint32 values[6] = 1; packed string names[2] = 2; required packed uint64 required_values[2] = 3; packed uint8 narrow[4] = 4; }",
    )
    .unwrap();
    let errors = analyze_schema(&schema).unwrap_err();
    assert_eq!(errors.errors().len(), 4);
    assert!(errors.errors().iter().all(|error| {
        error
            .message
            .contains("must use float32, float64, fixed32, or fixed64")
    }));
}

#[test]
fn float_defaults_are_rejected_until_a_canonical_literal_format_is_defined() {
    let schema =
        parse_schema("version 1; message Bad = 1 { optional float32 value = 1 [default = 0]; }")
            .unwrap();
    let errors = analyze_schema(&schema).unwrap_err();
    assert!(
        errors.errors()[0]
            .message
            .contains("float32 fields cannot declare defaults")
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
fn compatibility_treats_packed_kind_and_count_as_wire_identity() {
    let previous = analyze_schema(
        &parse_schema("version 1; message Control = 1 { packed float32 joints[6] = 1; }").unwrap(),
    )
    .unwrap();
    let same_shape = analyze_schema(
        &parse_schema(
            "version 2; message Control = 1 { packed float32 joints[6] = 1; optional float64 time = 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    check_compatibility(&previous, &same_shape).expect("adding an optional field is compatible");

    for source in [
        "version 2; message Control = 1 { packed float32 joints[7] = 1; }",
        "version 2; message Control = 1 { packed fixed32 joints[6] = 1; }",
        "version 2; message Control = 1 { repeated float32 joints = 1; }",
    ] {
        let current = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let errors = check_compatibility(&previous, &current).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains("changed wire identity"))
        );
    }
}

#[test]
fn compatibility_treats_narrow_integer_width_and_signedness_as_identity() {
    let previous = analyze_schema(
        &parse_schema(
            "version 1; message Value = 1 { optional uint8 unsigned_value = 1; optional int16 signed_value = 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    let compatible = analyze_schema(
        &parse_schema(
            "version 2; message Value = 1 { optional uint8 unsigned_value = 1; optional int16 signed_value = 2; optional uint16 added = 3; }",
        )
        .unwrap(),
    )
    .unwrap();
    check_compatibility(&previous, &compatible).expect("adding an optional narrow field is safe");

    for source in [
        "version 2; message Value = 1 { optional uint16 unsigned_value = 1; optional int16 signed_value = 2; }",
        "version 2; message Value = 1 { optional uint32 unsigned_value = 1; optional int16 signed_value = 2; }",
        "version 2; message Value = 1 { optional int8 unsigned_value = 1; optional int16 signed_value = 2; }",
        "version 2; message Value = 1 { optional uint8 unsigned_value = 1; optional int8 signed_value = 2; }",
        "version 2; message Value = 1 { optional uint8 unsigned_value = 1; optional int32 signed_value = 2; }",
    ] {
        let current = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let errors = check_compatibility(&previous, &current).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains("changed wire identity"))
        );
    }
}

#[test]
fn compatibility_treats_every_length_bound_change_as_incompatible() {
    let previous = analyze_schema(
        &parse_schema(
            "version 1; message Metadata = 1 { optional string<31> name = 1; repeated bytes<64> chunks = 2; }",
        )
        .unwrap(),
    )
    .unwrap();

    for source in [
        "version 2; message Metadata = 1 { optional string<32> name = 1; repeated bytes<64> chunks = 2; }",
        "version 2; message Metadata = 1 { optional string name = 1; repeated bytes<64> chunks = 2; }",
        "version 2; message Metadata = 1 { optional string<31> name = 1; repeated bytes chunks = 2; }",
    ] {
        let current = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let errors = check_compatibility(&previous, &current).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains("changed wire identity"))
        );
    }

    let unbounded = analyze_schema(
        &parse_schema(
            "version 1; message Metadata = 1 { optional string name = 1; repeated bytes chunks = 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    let newly_bounded = analyze_schema(
        &parse_schema(
            "version 2; message Metadata = 1 { optional string<31> name = 1; repeated bytes chunks = 2; }",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(check_compatibility(&unbounded, &newly_bounded).is_err());
}

#[test]
fn compatibility_rejects_required_field_addition_removal_and_cardinality_changes() {
    let previous = analyze_schema(
        &parse_schema(
            "version 1; message Control = 1 { required uint32 sequence = 1; required packed float32 joints[6] = 2; optional bool enabled = 3; }",
        )
        .unwrap(),
    )
    .unwrap();
    let compatible = analyze_schema(
        &parse_schema(
            "version 2; message Control = 1 { required uint32 sequence = 1; required packed float32 joints[6] = 2; optional bool enabled = 3; optional uint32 timestamp = 4; }",
        )
        .unwrap(),
    )
    .unwrap();
    check_compatibility(&previous, &compatible).expect("adding optional remains compatible");

    for source in [
        "version 2; message Control = 1 { optional uint32 sequence = 1; required packed float32 joints[6] = 2; optional bool enabled = 3; }",
        "version 2; message Control = 1 { required uint32 sequence = 1; packed float32 joints[6] = 2; optional bool enabled = 3; }",
        "version 2; message Control = 1 { required uint32 sequence = 1; required packed float32 joints[6] = 2; required bool enabled = 3; }",
    ] {
        let current = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let errors = check_compatibility(&previous, &current).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains("changed wire identity"))
        );
    }

    let removed = analyze_schema(
        &parse_schema(
            "version 2; message Control = 1 { reserved 1; required packed float32 joints[6] = 2; optional bool enabled = 3; }",
        )
        .unwrap(),
    )
    .unwrap();
    let errors = check_compatibility(&previous, &removed).unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error
            .message
            .contains("cannot be removed, even if reserved")
    }));

    for source in [
        "version 2; message Control = 1 { required uint32 sequence = 1; required packed float32 joints[6] = 2; optional bool enabled = 3; required uint64 timestamp = 4; }",
        "version 2; message Control = 1 { required uint32 sequence = 1; required packed float32 joints[6] = 2; optional bool enabled = 3; required packed fixed32 flags[2] = 4; }",
    ] {
        let current = analyze_schema(&parse_schema(source).unwrap()).unwrap();
        let errors = check_compatibility(&previous, &current).unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| { error.message.contains("new required field") })
        );
    }
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
    assert!(
        generated
            .bindings_header
            .contains("motor_api_status_handler_fn")
    );
    assert!(
        generated
            .bindings_header
            .contains("motor_api_status_send_unreliable")
    );
    assert!(
        generated
            .bindings_header
            .contains("motor_api_status_send_direct")
    );
    assert!(
        generated
            .bindings_source
            .contains("motor_api_dispatch_event")
    );
    assert!(generated.bindings_source.ends_with('\n'));
    assert!(!generated.bindings_source.ends_with("\n\n"));
    assert!(!generated.source.contains("wl_send_unreliable"));
}

#[test]
fn generated_codec_and_bindings_are_declaration_order_independent() {
    let first = analyze_schema(
        &parse_schema(
            "version 1; message Child = 2 { optional uint32 value = 1; } message Parent = 1 { optional Child child = 1; }",
        )
        .unwrap(),
    )
    .unwrap();
    let second = analyze_schema(
        &parse_schema(
            "version 1; message Parent = 1 { optional Child child = 1; } message Child = 2 { optional uint32 value = 1; }",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        generate_c(&first, "stable_api").unwrap(),
        generate_c(&second, "stable_api").unwrap()
    );
}

#[test]
fn generator_normalizes_acronyms_and_c_keywords() {
    let model = analyze_schema(
        &parse_schema("version 1; message HTTPStatus = 1 { optional uint32 switch = 1; }").unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "HTTP API").unwrap();
    assert!(generated.header.contains("struct http_status"));
    assert!(generated.header.contains("uint32_t switch_;"));
    assert!(generated.header.contains("HTTP_STATUS_MESSAGE_ID"));
    assert!(generated.source.contains("#include \"http_api.h\""));
}

#[test]
fn generator_emits_ieee_scalars_and_inline_packed_arrays() {
    let model = analyze_schema(
        &parse_schema(
            "version 1; message Control = 1 { optional float32 position = 1; optional float64 time = 2; packed float32 joints[6] = 3; packed fixed64 ticks[2] = 4; }",
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "control").unwrap();
    assert!(generated.header.contains("float position;"));
    assert!(generated.header.contains("double time;"));
    assert!(generated.header.contains("float joints[6];"));
    assert!(generated.header.contains("uint64_t ticks[2];"));
    assert!(generated.header.contains("IEEE-754 binary32"));
    assert!(generated.header.contains("IEEE-754 binary64"));
    assert!(generated.source.contains("WLC_PACKED"));
    assert!(generated.source.contains("memcpy(&bits32, value"));
}

proptest! {
    #[test]
    fn parser_never_panics_for_arbitrary_utf8(source in ".*") {
        let _ = parse_schema(&source);
    }

    #[test]
    fn full_width_numeric_defaults_are_accepted(value in any::<u64>()) {
        let source = format!(
            "version 1; message Value = 1 {{ optional uint64 field = 1 [default = {value}]; }}"
        );
        let schema = parse_schema(&source).expect("generated schema is syntactically valid");
        analyze_schema(&schema).expect("all u64 defaults must fit uint64");
    }

    #[test]
    fn signed_numeric_defaults_are_accepted(value in any::<i64>()) {
        let source = format!(
            "version 1; message Value = 1 {{ optional int64 field = 1 [default = {value}]; }}"
        );
        let schema = parse_schema(&source).expect("generated schema is syntactically valid");
        analyze_schema(&schema).expect("all i64 defaults must fit int64");
    }
}
