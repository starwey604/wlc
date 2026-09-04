use wlc::{
    analyze_binding_profile, analyze_schema, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

fn schema(source: &str) -> wlc::SemanticModel {
    analyze_schema(&parse_schema(source).unwrap()).unwrap()
}

fn profile(source: &str, schema: &wlc::SemanticModel) -> wlc::BindingProfileModel {
    analyze_binding_profile(&parse_binding_profile(source).unwrap(), schema).unwrap()
}

#[test]
fn runtime_namespace_rejects_schema_macro_type_and_service_collisions() {
    let type_collision =
        schema("version 1; message ControlRuntime = 1 { optional uint32 value = 1; }");
    let retained = profile(
        "profile version 1; latest ControlRuntime { delivery = unreliable; }",
        &type_collision,
    );
    let error = generate_runtime_c(&type_collision, &retained, "control").unwrap_err();
    assert!(error.0.contains("control_runtime_t"), "{error}");

    let assembly_type_collision =
        schema("version 1; message ControlRuntimeConfig = 1 { optional uint32 value = 1; }");
    let retained = profile(
        "profile version 1; latest ControlRuntimeConfig { delivery = unreliable; }",
        &assembly_type_collision,
    );
    let error = generate_runtime_c(&assembly_type_collision, &retained, "control").unwrap_err();
    assert!(error.0.contains("control_runtime_config_t"), "{error}");

    let macro_collision = schema(
        r#"version 1;
enum RuntimeNames = 1 { CONTROL_RUNTIME_OK = 0; }
message Value = 2 { optional uint32 value = 1; }
"#,
    );
    let retained = profile(
        "profile version 1; latest Value { delivery = unreliable; }",
        &macro_collision,
    );
    let error = generate_runtime_c(&macro_collision, &retained, "control").unwrap_err();
    assert!(error.0.contains("CONTROL_RUNTIME_OK"), "{error}");

    let service_collision = schema(
        r#"version 1;
enum Status = 1 { SUCCESS = 0; }
message Request = 2 { optional uint32 operation_id = 1; }
message Response = 3 {
  optional uint32 operation_id = 1;
  optional Status status = 2;
}
message ControlStartRpc = 4 { optional uint32 value = 1; }
"#,
    );
    let service_profile = profile(
        r#"profile version 1;
rpc Start {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
        &service_collision,
    );
    let error = generate_runtime_c(&service_collision, &service_profile, "control").unwrap_err();
    assert!(error.0.contains("control_start_rpc_t"), "{error}");

    let reserved_member_profile = profile(
        r#"profile version 1;
rpc RpcClient {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
        &service_collision,
    );
    let error =
        generate_runtime_c(&service_collision, &reserved_member_profile, "control").unwrap_err();
    assert!(error.0.contains("rpc_client"), "{error}");
}

#[test]
fn runtime_revalidates_profile_models_against_the_supplied_schema() {
    let retained_a = schema("version 1; message State = 1 { optional uint32 sequence = 1; }");
    let retained_profile = profile(
        "profile version 1; latest State { delivery = unreliable; }",
        &retained_a,
    );
    let retained_b = schema(
        r#"version 1;
message State = 1 {
  optional uint32 sequence = 1;
  optional bytes borrowed = 2;
}
"#,
    );
    let error = generate_runtime_c(&retained_b, &retained_profile, "state").unwrap_err();
    assert!(error.0.contains("borrowed or caller-owned"), "{error}");

    let rpc_a = schema(
        r#"version 1;
enum Status = 1 { SUCCESS = 0; }
message Request = 2 { optional uint32 operation_id = 1; }
message Response = 3 {
  optional uint32 operation_id = 1;
  optional Status status = 2;
}
"#,
    );
    let rpc_profile = profile(
        r#"profile version 1;
rpc Start {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
        &rpc_a,
    );
    let rpc_b = schema(
        r#"version 1;
enum Status = 1 { SUCCESS = 0; }
message Request = 2 { optional uint32 operation_id = 9; }
message Response = 3 {
  optional uint32 operation_id = 1;
  optional int32 status = 2;
}
"#,
    );
    let error = generate_runtime_c(&rpc_b, &rpc_profile, "rpc_api").unwrap_err();
    assert!(
        error
            .0
            .contains("operation field `Request.operation_id` number 1"),
        "{error}"
    );
}

#[test]
fn runtime_generation_is_deterministic_and_emits_separate_identities() {
    let schema = schema(
        r#"version 1;
message State = 1 { optional uint32 sequence = 1; }
"#,
    );
    let profile = profile(
        "profile version 1; fifo State { delivery = reliable; }",
        &schema,
    );
    let first = generate_runtime_c(&schema, &profile, "stable_api").unwrap();
    let second = generate_runtime_c(&schema, &profile, "stable_api").unwrap();
    assert_eq!(first, second);
    assert!(first.header.contains("STABLE_API_SCHEMA_IDENTITY"));
    assert!(first.header.contains("STABLE_API_BINDING_PROFILE_IDENTITY"));
    assert!(first.header.contains("STABLE_API_IDENTITY_ALGORITHM"));
    assert!(
        first
            .header
            .contains("STABLE_API_RUNTIME_CODEGEN_ABI_VERSION")
    );
    assert!(first.header.contains("#include <wirelink/fifo.h>"));
    assert!(first.header.contains("#include <wirelink/pump.h>"));
    assert!(!first.header.contains("#include <wirelink/latest.h>"));
    assert!(!first.header.contains("#include <wirelink/rpc.h>"));
    assert!(
        first
            .header
            .contains("stable_api_state_fifo_view_t *out_view")
    );
    assert!(
        first
            .header
            .contains("stable_api_state_fifo_release(stable_api_runtime_t *runtime")
    );
    assert!(
        first
            .source
            .contains("stable_api_state_fifo_acquire(stable_api_runtime_t *runtime")
    );
    assert!(first.source.contains(
        "int failure = lease.value_size < sizeof(state_t) ? WL_ERR_BUF_TOO_SMALL : WL_ERR_INVALID_STATE;\n    int release_result = wl_fifo_read_release"
    ));
    assert!(
        first
            .header
            .contains("stable_api_runtime_retained_detail_t retained;")
    );
    assert!(
        !first
            .header
            .contains("stable_api_runtime_rpc_detail_t rpc;")
    );
    assert!(first.header.contains("result.event_consumed"));
    assert!(first.header.contains("uint8_t event_consumed;"));
    assert!(first.source.contains("result.event_consumed = 1U;"));
    assert!(first.header.contains("stable_api_runtime_pump_hooks"));
    assert!(
        first
            .source
            .contains("return result.event_consumed != 0U ? WL_PUMP_EVENT_CONSUMED")
    );
    assert!(first.source.contains("goto init_failed;"));
    assert!(
        first
            .source
            .contains("init_failed:\n  memset(instance, 0, sizeof(*instance));")
    );
}

#[test]
fn unreliable_client_and_reliable_response_paths_are_generated_explicitly() {
    let schema = schema(
        r#"version 1;
message Request = 1 { optional uint32 operation_id = 1; }
message Response = 2 {
  optional uint32 operation_id = 1;
  optional int32 status = 2;
}
"#,
    );
    let profile = profile(
        r#"profile version 1;
rpc Local {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = unreliable;
  response_delivery = reliable;
}
"#,
        &schema,
    );
    let generated = generate_runtime_c(&schema, &profile, "local_rpc").unwrap();
    assert!(generated.header.contains("#include <wirelink/rpc.h>"));
    assert!(!generated.header.contains("#include <wirelink/fifo.h>"));
    assert!(!generated.header.contains("#include <wirelink/latest.h>"));
    assert!(
        generated
            .header
            .contains("local_rpc_runtime_rpc_detail_t rpc;")
    );
    assert!(
        !generated
            .header
            .contains("runtime_retained_detail_t retained;")
    );
    assert!(
        generated
            .source
            .contains("wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)")
    );
    assert!(
        generated
            .header
            .contains("local_rpc_runtime_poll_result_t *out_result")
    );
    assert!(
        generated
            .header
            .contains("local_rpc_runtime_get_deadline_hint")
    );
    assert!(generated.header.contains("local_rpc_local_client_inspect"));
    assert!(generated.header.contains("local_rpc_local_client_decode"));
    assert!(generated.source.contains("local_rpc_local_client_release"));
    assert!(
        generated
            .source
            .contains("wl_rpc_client_begin_with_id(runtime->rpc_client, operation_id")
    );
    assert!(
        generated
            .source
            .contains("response->has_operation_id = had_operation_id;")
    );
    assert!(generated.source.contains(
        "wl_send_reliable(ctx, response.identity.response_message_id, response.response_data"
    ));
    assert!(generated.source.contains("reliable_response = 1U;"));
}

#[test]
fn runtime_generation_accepts_required_rpc_mappings() {
    let schema = schema(
        r#"version 1;
enum Status = 1 { OK = 0; }
message Request = 2 { required uint32 operation_id = 1; }
message Response = 3 {
  required uint32 operation_id = 1;
  required Status status = 2;
}
"#,
    );
    let profile = profile(
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
        &schema,
    );
    let generated = generate_runtime_c(&schema, &profile, "required_rpc").unwrap();
    assert!(
        generated
            .source
            .contains("request->has_operation_id = true;")
    );
    assert!(generated.source.contains("response->has_status = true;"));
}
