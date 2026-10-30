use wlc::{analyze_binding_profile, analyze_schema, parse_binding_profile, parse_schema};
const SCHEMA: &str = "version 1; message Input @id(1) { required int32 value @id(1); } message Output @id(2) { required int32 value @id(1); }";
#[test]
fn annotated_delivery_matches_current_defaults_and_reliable_is_rejected() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let mut models = Vec::new();
    for properties in [
        "request=Input; response=Output;",
        "request=Input @delivery(unreliable); response=Output @delivery(unreliable);",
        "request=Input;response=Output;request_delivery=unreliable;response_delivery=unreliable;",
    ] {
        models.push(
            analyze_binding_profile(
                &parse_binding_profile(&format!("profile version 2; rpc Echo {{ {properties} }}"))
                    .unwrap(),
                &schema,
            )
            .unwrap(),
        );
    }
    assert_eq!(models[0], models[1]);
    assert_eq!(models[1], models[2]);
    for properties in [
        "request=Input @delivery(reliable);response=Output;",
        "request=Input;response=Output @delivery(reliable);",
    ] {
        let parsed =
            parse_binding_profile(&format!("profile version 2; rpc Echo {{ {properties} }}"))
                .unwrap();
        assert!(analyze_binding_profile(&parsed, &schema).is_err());
    }
}
#[test]
fn duplicate_policies_are_errors_in_both_orders_even_when_equal() {
    for body in [
        "request = Input @delivery(reliable) @delivery(reliable); response = Output;",
        "request = Input @delivery(reliable); request_delivery = reliable; response = Output;",
        "request_delivery = reliable; request = Input @delivery(reliable); response = Output;",
        "request = Input; response = Output @delivery(unreliable); response_delivery = reliable;",
        "response_delivery = unreliable; request = Input; response = Output @delivery(reliable);",
    ] {
        let error = parse_binding_profile(&format!("profile version 2; rpc Echo {{ {body} }}"))
            .unwrap_err();
        assert!(error.message.contains("duplicate"), "{error}");
    }
}

#[test]
fn malformed_or_misplaced_attributes_have_diagnostics() {
    for body in [
        "request = Input @delibery(reliable); response = Output;",
        "request = Input @delivery(); response = Output;",
        "request = Input @delivery reliable; response = Output;",
        "request = Input @delivery(reliable; response = Output;",
        "request = Input; response = Output; response_status = status @delivery(reliable);",
    ] {
        assert!(
            parse_binding_profile(&format!("profile version 2; rpc Echo {{ {body} }}")).is_err()
        );
    }
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile = parse_binding_profile(
        "profile version 2; rpc Echo {
        request = Input @delivery(magic); response = Output; }",
    )
    .unwrap();
    assert!(
        analyze_binding_profile(&profile, &schema)
            .unwrap_err()
            .errors()
            .iter()
            .any(|e| e.message.contains("delivery"))
    );
    // No implicit change to retained routes or business schema grammar.
    assert!(parse_binding_profile("profile version 2; latest Input {}").is_err());
    assert!(parse_schema("version 1; message Input @id(1) @delivery(reliable) {}").is_err());
}
