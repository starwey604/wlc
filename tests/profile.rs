use wlc::{
    IDENTITY_ALGORITHM, analyze_binding_profile, analyze_schema, binding_profile_identity,
    parse_binding_profile, parse_schema,
    profile::BindingDeclaration,
    profile_semantic::{DeliveryPolicy, RetainedRouteKind, RpcStatusDomain},
    schema_identity,
};

const SCHEMA: &str = r#"
version 1;

enum OperationStatus = 1 {
  OK = 0;
  REJECTED = 1;
}

enum StatusWithoutSuccess = 2 {
  FAILED = 1;
}

message InlineChild = 10 {
  optional uint32 value = 1;
}

message Control = 11 {
  packed float32 values[6] = 1;
  optional InlineChild child = 2;
}

message Event = 12 {
  optional fixed32 code = 1;
}

message Borrowed = 13 {
  optional bytes payload = 1;
}

message Repeated = 14 {
  repeated uint32 samples = 1;
}

message NestedBorrowed = 15 {
  optional Borrowed child = 1;
}

message StartRequest = 20 {
  optional uint32 operation_id = 1;
  optional bytes arguments = 2;
}

message StartResponse = 21 {
  optional uint32 operation_id = 1;
  optional OperationStatus status = 2;
  optional bytes result = 3;
}

message WrongRequest = 22 {
  optional uint64 operation_id = 1;
}

message WrongResponse = 23 {
  optional uint32 operation_id = 1;
  optional uint32 status = 2;
}

message NoSuccessResponse = 24 {
  optional uint32 operation_id = 1;
  optional StatusWithoutSuccess status = 2;
}
"#;

const PROFILE: &str = r#"
profile version 1;

latest Control {
  delivery = unreliable;
}

fifo Event {
  delivery = reliable;
}

rpc Start {
  request = StartRequest;
  response = StartResponse;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#;

fn schema_model() -> wlc::SemanticModel {
    analyze_schema(&parse_schema(SCHEMA).expect("schema parses")).expect("schema resolves")
}

#[test]
fn parses_all_profile_v1_bindings_without_touching_schema_grammar() {
    let profile = parse_binding_profile(PROFILE).expect("profile parses");
    assert_eq!(profile.version.value, 1);
    assert_eq!(profile.bindings.len(), 3);
    assert!(matches!(profile.bindings[0], BindingDeclaration::Latest(_)));
    assert!(matches!(profile.bindings[1], BindingDeclaration::Fifo(_)));
    assert!(matches!(profile.bindings[2], BindingDeclaration::Rpc(_)));

    assert!(
        parse_schema(PROFILE).is_err(),
        "a profile is not a .wl schema"
    );
    parse_schema("version 1; message profile = 1 { optional uint32 rpc = 1; }")
        .expect("profile vocabulary stays legal in frozen .wl identifiers");
}

#[test]
fn resolves_and_canonically_orders_route_and_rpc_metadata() {
    let profile = parse_binding_profile(PROFILE).unwrap();
    let model = analyze_binding_profile(&profile, &schema_model()).unwrap();
    assert_eq!(model.version, 1);
    assert_eq!(model.retained_routes.len(), 2);
    assert_eq!(model.retained_routes[0].message_name, "Control");
    assert_eq!(model.retained_routes[0].message_id, 11);
    assert_eq!(model.retained_routes[0].kind, RetainedRouteKind::Latest);
    assert_eq!(
        model.retained_routes[0].delivery,
        DeliveryPolicy::Unreliable
    );
    assert_eq!(model.retained_routes[1].message_name, "Event");
    assert_eq!(model.retained_routes[1].kind, RetainedRouteKind::Fifo);

    let service = &model.rpc_services[0];
    assert_eq!(service.name, "Start");
    assert_eq!(service.request_id, 20);
    assert_eq!(service.response_id, 21);
    assert_eq!(service.request_operation_id.as_ref().unwrap().number, 1);
    assert_eq!(service.response_operation_id.as_ref().unwrap().number, 1);
    assert_eq!(service.response_status.as_ref().unwrap().number, 2);
    assert_eq!(
        service.status_domain,
        Some(RpcStatusDomain::Enum {
            name: "OperationStatus".to_owned(),
            id: 1
        })
    );
    assert_eq!(service.request_delivery, DeliveryPolicy::Reliable);
    assert_eq!(service.response_delivery, DeliveryPolicy::Reliable);
}

#[test]
fn profile_parser_reports_unknown_duplicate_and_missing_rpc_properties() {
    let unknown = parse_binding_profile(
        "profile version 1; rpc Bad { request = StartRequest; mystery = value; }",
    )
    .unwrap_err();
    assert_eq!(unknown.span.column, 54);
    assert!(unknown.message.contains("unknown RPC property `mystery`"));

    let duplicate = parse_binding_profile(
        "profile version 1; rpc Bad { request = StartRequest; request = StartRequest; }",
    )
    .unwrap_err();
    assert!(
        duplicate
            .message
            .contains("duplicate RPC property `request`")
    );

    let missing = parse_binding_profile(
        "profile version 1; rpc Bad { request = StartRequest; response = StartResponse; }",
    )
    .unwrap_err();
    assert!(
        missing
            .message
            .contains("missing RPC property `request_delivery`")
    );
}

#[test]
fn rejects_unsupported_profile_versions_and_invalid_delivery() {
    let profile =
        parse_binding_profile("profile version 2; latest Control { delivery = best_effort; }")
            .unwrap();
    let errors = analyze_binding_profile(&profile, &schema_model()).unwrap_err();
    assert_eq!(errors.errors().len(), 2);
    assert!(errors.errors().iter().any(|error| {
        error
            .message
            .contains("unsupported binding profile version 2")
    }));
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message.contains("invalid delivery `best_effort`"))
    );
}

#[test]
fn retained_routes_reject_borrowed_repeated_and_transitively_borrowed_messages() {
    for (message, expected_path) in [
        ("Borrowed", "Borrowed.payload"),
        ("Repeated", "Repeated.samples"),
        ("NestedBorrowed", "NestedBorrowed.child.Borrowed.payload"),
    ] {
        let source = format!("profile version 1; latest {message} {{ delivery = unreliable; }}");
        let profile = parse_binding_profile(&source).unwrap();
        let errors = analyze_binding_profile(&profile, &schema_model()).unwrap_err();
        assert_eq!(errors.errors().len(), 1, "{message}");
        assert!(errors.errors()[0].message.contains(expected_path));
        assert!(
            errors.errors()[0]
                .message
                .contains("borrowed or caller-owned storage")
        );
    }
}

#[test]
fn rejects_duplicate_retained_routes_and_rpc_role_collisions() {
    let duplicate_route = parse_binding_profile(
        "profile version 1; latest Control { delivery = unreliable; } fifo Control { delivery = reliable; }",
    )
    .unwrap();
    let errors = analyze_binding_profile(&duplicate_route, &schema_model()).unwrap_err();
    assert!(
        errors.errors()[0]
            .message
            .contains("already has a retained")
    );

    let route_and_rpc_source = PROFILE.replacen(
        "profile version 1;",
        "profile version 1; latest StartRequest { delivery = unreliable; }",
        1,
    );
    let route_and_rpc = parse_binding_profile(&route_and_rpc_source).unwrap();
    let errors = analyze_binding_profile(&route_and_rpc, &schema_model()).unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error
            .message
            .contains("RPC request message `StartRequest` already has a retained")
    }));

    let reused_role = PROFILE.replace("rpc Start {", "rpc Start {\n")
        + r#"
rpc Other {
  request = StartRequest;
  response = WrongResponse;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = unreliable;
}
"#;
    let errors = analyze_binding_profile(
        &parse_binding_profile(&reused_role).unwrap(),
        &schema_model(),
    )
    .unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error
            .message
            .contains("already the request of RPC service `Start`")
    }));
}

#[test]
fn rpc_requires_singular_uint32_ids_and_an_int32_status_domain_with_zero_success() {
    let cases = [
        (
            "WrongRequest",
            "StartResponse",
            "operation_id",
            "must map to an optional or required uint32 field",
        ),
        (
            "StartRequest",
            "WrongResponse",
            "operation_id",
            "status must map to an optional or required int32 or enum field",
        ),
        (
            "StartRequest",
            "NoSuccessResponse",
            "operation_id",
            "must declare numeric value zero",
        ),
    ];
    for (request, response, operation_field, expected) in cases {
        let source = format!(
            r#"profile version 1;
rpc Test {{
  request = {request};
  response = {response};
  request_operation_id = {operation_field};
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = unreliable;
}}"#
        );
        let errors =
            analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema_model())
                .unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains(expected)),
            "expected `{expected}` in {:?}",
            errors.errors()
        );
    }
}

#[test]
fn rpc_accepts_required_operation_and_status_fields() {
    let schema = analyze_schema(
        &parse_schema(
            r#"version 1;
enum Status = 1 { OK = 0; FAILED = 1; }
message Request = 2 { required uint32 operation_id = 1; }
message Response = 3 {
  required uint32 operation_id = 1;
  required Status status = 2;
}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let profile = parse_binding_profile(
        r#"profile version 1;
rpc RequiredFields {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
    )
    .unwrap();
    analyze_binding_profile(&profile, &schema).expect("required RPC fields are singular");
}

#[test]
fn rejects_unknown_messages_fields_and_enum_bindings() {
    for (source, expected) in [
        (
            "profile version 1; latest Missing { delivery = reliable; }",
            "unknown message `Missing`",
        ),
        (
            "profile version 1; latest OperationStatus { delivery = reliable; }",
            "is an enum; a binding requires a message",
        ),
        (
            r#"profile version 1;
rpc Bad {
  request = StartRequest;
  response = StartResponse;
  request_operation_id = missing;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}"#,
            "message `StartRequest` has no field `missing`",
        ),
    ] {
        let errors =
            analyze_binding_profile(&parse_binding_profile(source).unwrap(), &schema_model())
                .unwrap_err();
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.message.contains(expected))
        );
    }
}

#[test]
fn semantic_identities_are_stable_and_have_reviewed_golden_values() {
    let schema = schema_model();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    assert_eq!(IDENTITY_ALGORITHM, "fnv1a64-v1");
    assert_eq!(schema_identity(&schema), 0x792a_0fdd_6f09_b0be);
    assert_eq!(binding_profile_identity(&profile), 0xeadf_5664_4de5_8304);
}

#[test]
fn identities_ignore_source_order_but_cover_exact_semantics_and_policy() {
    let first_schema = analyze_schema(
        &parse_schema(
            "version 7; enum State = 1 { OFF = 0; ON = 1; } message Sample = 2 { optional State state = 2 [default = 0]; optional uint32 sequence = 1; } message Ack = 3 { optional uint32 operation_id = 1; optional int32 status = 2; } message Request = 4 { optional uint32 operation_id = 1; }",
        )
        .unwrap(),
    )
    .unwrap();
    let reordered_schema = analyze_schema(
        &parse_schema(
            "version 7; message Request = 4 { optional uint32 operation_id = 1; } message Ack = 3 { optional int32 status = 2; optional uint32 operation_id = 1; } message Sample = 2 { optional uint32 sequence = 1; optional State state = 2 [default = 0]; } enum State = 1 { ON = 1; OFF = 0; }",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema_identity(&first_schema),
        schema_identity(&reordered_schema)
    );

    let changed_revision = analyze_schema(
        &parse_schema(
            "version 8; enum State = 1 { OFF = 0; ON = 1; } message Sample = 2 { optional State state = 2 [default = 0]; optional uint32 sequence = 1; } message Ack = 3 { optional uint32 operation_id = 1; optional int32 status = 2; } message Request = 4 { optional uint32 operation_id = 1; }",
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(
        schema_identity(&first_schema),
        schema_identity(&changed_revision)
    );

    let profile_a = analyze_binding_profile(
        &parse_binding_profile(
            r#"profile version 1;
fifo Sample { delivery = unreliable; }
rpc Read {
  request = Request;
  response = Ack;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
        )
        .unwrap(),
        &first_schema,
    )
    .unwrap();
    let profile_b = analyze_binding_profile(
        &parse_binding_profile(
            r#"profile version 1;
rpc Read {
  response_status = status;
  response = Ack;
  response_delivery = reliable;
  request_operation_id = operation_id;
  request_delivery = reliable;
  request = Request;
  response_operation_id = operation_id;
}
fifo Sample { delivery = unreliable; }
"#,
        )
        .unwrap(),
        &reordered_schema,
    )
    .unwrap();
    assert_eq!(
        binding_profile_identity(&profile_a),
        binding_profile_identity(&profile_b)
    );

    let changed_policy = analyze_binding_profile(
        &parse_binding_profile(
            r#"profile version 1;
fifo Sample { delivery = reliable; }
rpc Read {
  request = Request;
  response = Ack;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
        )
        .unwrap(),
        &first_schema,
    )
    .unwrap();
    assert_ne!(
        binding_profile_identity(&profile_a),
        binding_profile_identity(&changed_policy)
    );
}

#[test]
fn schema_identity_distinguishes_required_cardinality() {
    let optional = analyze_schema(
        &parse_schema("version 1; message Value = 1 { optional uint32 field = 1; }").unwrap(),
    )
    .unwrap();
    let required = analyze_schema(
        &parse_schema("version 1; message Value = 1 { required uint32 field = 1; }").unwrap(),
    )
    .unwrap();
    assert_ne!(schema_identity(&optional), schema_identity(&required));
}

#[test]
fn schema_identity_distinguishes_narrow_integer_types() {
    let identities = ["uint8", "uint16", "int8", "int16"]
        .into_iter()
        .map(|ty| {
            let source = format!("version 1; message Value = 1 {{ optional {ty} field = 1; }}");
            let model = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
            schema_identity(&model)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 4);
}

#[test]
fn schema_identity_distinguishes_bounded_lengths_without_renumbering_unbounded_types() {
    let identities = ["string", "string<31>", "string<32>", "bytes<31>"]
        .into_iter()
        .map(|ty| {
            let source = format!("version 1; message Value = 1 {{ optional {ty} field = 1; }}");
            let model = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
            schema_identity(&model)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 4);
}
