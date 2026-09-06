use wlc::{
    ManifestArtifact, analyze_binding_profile, analyze_schema, binding_profile_identity,
    generate_c, generate_codegen_manifest, generate_runtime_c, parse_binding_profile, parse_schema,
    profile_semantic::DeliveryPolicy,
};

const SCHEMA: &str = "version 1;
message Input @id(1) { required int32 value @id(1); }
message Output @id(2) { required int32 value @id(1); }";

#[test]
fn omitted_annotated_and_legacy_delivery_generate_identical_artifacts() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let mut artifacts = Vec::new();
    for body in [
        "request = Input; response = Output;",
        "request = Input @delivery(reliable); response = Output @delivery(reliable);",
        "request = Input; response = Output; request_delivery = reliable; response_delivery = reliable;",
    ] {
        let parsed =
            parse_binding_profile(&format!("profile version 1; rpc Echo {{ {body} }}")).unwrap();
        let model = analyze_binding_profile(&parsed, &schema).unwrap();
        let identity = binding_profile_identity(&model);
        let codec = generate_c(&schema, "echo").unwrap();
        let runtime = generate_runtime_c(&schema, &model, "echo").unwrap();
        let manifest = generate_codegen_manifest(
            "echo",
            &schema,
            Some(identity),
            &[
                ManifestArtifact {
                    path: "echo_runtime.h",
                    contents: runtime.header.as_bytes(),
                },
                ManifestArtifact {
                    path: "echo_runtime.c",
                    contents: runtime.source.as_bytes(),
                },
            ],
        );
        artifacts.push((identity, codec, runtime, manifest));
    }
    assert_eq!(artifacts[0], artifacts[1]);
    assert_eq!(artifacts[0], artifacts[2]);
}

#[test]
fn directions_are_independent_and_whitespace_is_allowed() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    for (request, response) in [
        ("reliable", "reliable"),
        ("unreliable", "reliable"),
        ("reliable", "unreliable"),
        ("unreliable", "unreliable"),
    ] {
        let source = format!(
            "profile version 1; rpc Echo {{
            request = Input @ // comment\n delivery ({request});
            response = Output @delivery({response}); }}"
        );
        let model =
            analyze_binding_profile(&parse_binding_profile(&source).unwrap(), &schema).unwrap();
        let legacy = format!(
            "profile version 1; rpc Echo {{ request = Input; response = Output;
            request_delivery = {request}; response_delivery = {response}; }}"
        );
        let legacy =
            analyze_binding_profile(&parse_binding_profile(&legacy).unwrap(), &schema).unwrap();
        assert_eq!(model, legacy);
    }
    let source =
        "profile version 1; rpc Echo { request = Input @delivery(unreliable); response = Output; }";
    let model = analyze_binding_profile(&parse_binding_profile(source).unwrap(), &schema).unwrap();
    assert_eq!(
        model.rpc_services[0].request_delivery,
        DeliveryPolicy::Unreliable
    );
    assert_eq!(
        model.rpc_services[0].response_delivery,
        DeliveryPolicy::Reliable
    );
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
        let error = parse_binding_profile(&format!("profile version 1; rpc Echo {{ {body} }}"))
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
            parse_binding_profile(&format!("profile version 1; rpc Echo {{ {body} }}")).is_err()
        );
    }
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile = parse_binding_profile(
        "profile version 1; rpc Echo {
        request = Input @delivery(magic); response = Output; }",
    )
    .unwrap();
    assert!(
        analyze_binding_profile(&profile, &schema)
            .unwrap_err()
            .errors()
            .iter()
            .any(|e| e.message.contains("invalid delivery `magic`"))
    );
    // No implicit change to retained routes or business schema grammar.
    assert!(parse_binding_profile("profile version 1; latest Input {}").is_err());
    assert!(parse_schema("version 1; message Input @id(1) @delivery(reliable) {}").is_err());
}
