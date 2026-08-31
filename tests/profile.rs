use wlc::{
    analyze_binding_profile, analyze_schema, parse_binding_profile, parse_schema,
    profile::BindingDeclaration,
    profile_semantic::{DeliveryPolicy, RetainedRouteKind, RpcStatusDomain},
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
    assert_eq!(service.request_operation_id.number, 1);
    assert_eq!(service.response_operation_id.number, 1);
    assert_eq!(service.response_status.number, 2);
    assert_eq!(
        service.status_domain,
        RpcStatusDomain::Enum {
            name: "OperationStatus".to_owned(),
            id: 1
        }
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
            .contains("missing RPC property `request_operation_id`")
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
fn rpc_requires_optional_uint32_ids_and_an_int32_status_domain_with_zero_success() {
    let cases = [
        (
            "WrongRequest",
            "StartResponse",
            "operation_id",
            "must map to an optional uint32 field",
        ),
        (
            "StartRequest",
            "WrongResponse",
            "operation_id",
            "status must map to an optional int32 or enum field",
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
