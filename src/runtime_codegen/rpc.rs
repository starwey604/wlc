// SPDX-License-Identifier: Apache-2.0
use crate::codegen::{c_identifier, type_name, upper_snake};
use crate::profile_semantic::{BindingProfileModel, DeliveryPolicy, RpcService};
use std::fmt::Write;

pub(super) fn emit_rpc_header_types(
    output: &mut String,
    codec_module: &str,
    module: &str,
    service: &RpcService,
) {
    if service.is_managed() {
        output.push_str(&crate::managed_rpc_codegen::header_types(
            module,
            codec_module,
            service,
        ));
        return;
    }
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    write!(
        output,
        "/* The decoded request and its borrowed fields are valid only for the\n * callback. Copy server_request for asynchronous completion. Its generation\n * prevents a late completion from targeting a reused request identity. A\n * nonzero return abandons this exact pending operation. */\ntypedef int32_t (*{module}_{service_name}_rpc_request_handler_fn)(void *user_data, const {request}_t *request, const wl_rpc_server_request_t *server_request, wl_delivery_t delivery);\ntypedef struct {{\n  {request}_t *request_scratch;\n  {module}_{service_name}_rpc_request_handler_fn request_handler;\n  void *user_data;\n}} {module}_{service_name}_rpc_t;\n\n"
    )
    .unwrap();
}

pub(super) fn emit_rpc_header_functions(output: &mut String, module: &str, service: &RpcService) {
    if service.is_managed() {
        output.push_str(&crate::managed_rpc_codegen::header_functions(
            module, service,
        ));
        return;
    }
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* Allocates, encodes, and submits atomically from the caller's view. A\n * present nonzero request operation ID is used exactly, allowing an explicit\n * retry to address the server's bounded replay cache; absent or zero selects an\n * automatically allocated ID. The const request is copied into runtime-owned\n * encode scratch before operation ID injection. A local encode/submit failure\n * releases the allocated RPC slot and returns operation_id zero. */\n{module}_runtime_result_t {module}_{service_name}_client_start(wl_ctx_t *ctx, {module}_runtime_t *runtime, const {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms);\n/* Nonblocking inspection returns generic metadata for this service. */\nwl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client);\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id);\n\n/* server_request is copied from the request callback and uniquely scopes this\n * execution generation. Completion copies the const response into runtime-owned\n * encode scratch before injecting operation ID/status. runtime_service() later\n * submits the cached response bytes. */\n{module}_runtime_result_t {module}_{service_name}_server_complete({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, const {response}_t *response, wl_time_ms_t now_ms);\n{module}_runtime_result_t {module}_{service_name}_server_reject({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, const {response}_t *response, wl_time_ms_t now_ms);\n\n"
    )
    .unwrap();
}

pub(super) fn emit_rpc_runtime_progress_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    profile: &BindingProfileModel,
) {
    let server_expiry = if profile.has_replay() {
        "  wl_rpc_server_expiry_t server_expiry = {0};\n"
    } else {
        ""
    };
    let poll_history = if profile.has_replay() {
        "    result = wl_rpc_server_poll(runtime->rpc_server, now_ms, &server_expiry);\n    if (result != WL_RPC_OK) return result;\n    out_result->server_cache_expired = server_expiry.cache_expired;\n".to_owned()
    } else {
        String::new()
    };
    let server_deadline = if profile.has_replay() {
        "wl_rpc_server_get_deadline_hint"
    } else {
        "wl_rpc_server_get_pending_deadline_hint"
    };
    write!(output, "static uint8_t {module}_rpc_response_writable(const wl_ctx_t *ctx, const {module}_runtime_t *runtime) {{\n  uint16_t message_id;\n  if (runtime->rpc_response_retry_blocked &&\n      runtime->rpc_response_retry_generation == wl_link_resource_generation(ctx)) return 0U;\n  if (runtime->rpc_server == NULL || wl_rpc_server_response_peek(runtime->rpc_server, &message_id) != WL_RPC_OK) return 0U;\n  switch (message_id) {{\n").unwrap();
    for service in &profile.rpc_services {
        let response = upper_snake(&service.response_name);
        let delivery = match service.response_delivery {
            DeliveryPolicy::Reliable => "WL_DELIVERY_RELIABLE",
            DeliveryPolicy::Unreliable => "WL_DELIVERY_UNRELIABLE",
        };
        writeln!(
            output,
            "    case {response}_MESSAGE_ID: return wl_tx_can_submit(ctx, {delivery});"
        )
        .unwrap();
    }
    output
        .push_str("    default: return wl_tx_can_submit(ctx, WL_DELIVERY_UNRELIABLE);\n  }\n}\n\n");
    write!(
        output,
        "wl_rpc_err_t {module}_runtime_peer_observe(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint64_t peer_session_id, wl_rpc_peer_observation_t *out_observation) {{\n  {module}_peer_cancel_context_t cancel = {{ctx, runtime}};\n  wl_rpc_err_t result;\n  if (out_observation != NULL) memset(out_observation, 0, sizeof(*out_observation));\n  if (ctx == NULL || runtime == NULL || runtime->rpc_server == NULL || peer_session_id == 0U || out_observation == NULL) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_peer_observe(runtime->rpc_server, &runtime->rpc_peer, peer_session_id, {module}_runtime_cancel_peer_tx, &cancel, out_observation);\n  if (result == WL_RPC_OK && out_observation->changed != 0U) runtime->rpc_peer_observation = *out_observation;\n  return result;\n}}\n\n"
    )
    .unwrap();
    write!(
        output,
        "wl_rpc_err_t {module}_runtime_peer_observation_take({module}_runtime_t *runtime, wl_rpc_peer_observation_t *out_observation) {{\n  if (out_observation != NULL) memset(out_observation, 0, sizeof(*out_observation));\n  if (runtime == NULL || out_observation == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_peer_observation.changed == 0U) return WL_RPC_ERR_NOT_FOUND;\n  *out_observation = runtime->rpc_peer_observation;\n  memset(&runtime->rpc_peer_observation, 0, sizeof(runtime->rpc_peer_observation));\n  return WL_RPC_OK;\n}}\n\n"
    )
    .unwrap();
    write!(
        output,
        "wl_rpc_err_t {module}_runtime_poll({module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_poll_result_t *out_result) {{\n  wl_rpc_err_t result;\n{server_expiry}  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_poll(runtime->rpc_client, now_ms, &out_result->client_timed_out);\n    if (result != WL_RPC_OK) return result;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = wl_rpc_server_expired_acquire(runtime->rpc_server, now_ms, &out_result->server_expired_request);\n    if (result == WL_RPC_OK) out_result->server_pending_expired = 1U;\n    else if (result != WL_RPC_ERR_NOT_FOUND) return result;\n{poll_history}  }}\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_service(wl_ctx_t *ctx, {module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_service_result_t *out_result) {{\n  wl_rpc_server_response_t response = {{0}};\n  wl_rpc_err_t result;\n  uint8_t reliable_response = 0U;\n  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (ctx == NULL || runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  out_result->response = {module}_runtime_result(NULL);\n  if (runtime->rpc_retiring_tx != 0U) {{\n    wl_tx_result_t ignored;\n    int retired = wl_tx_can_take(ctx, runtime->rpc_retiring_tx) ? wl_tx_take(ctx, runtime->rpc_retiring_tx, &ignored) : WL_ERR_BUSY;\n    if (retired == WL_OK || retired == WL_ERR_NOT_FOUND) runtime->rpc_retiring_tx = 0U;\n  }}\n  wl_rpc_deadline_hint_t timer;\n  result = {module}_runtime_get_deadline_hint(runtime, now_ms, &timer);\n  if (result != WL_RPC_OK) return result;\n  if (timer.next_deadline_ms == 0U) {{\n    result = {module}_runtime_poll(runtime, now_ms, &out_result->deadlines);\n    if (result != WL_RPC_OK) return result;\n  }}\n  if (!{module}_rpc_response_writable(ctx, runtime)) return WL_RPC_OK;\n  runtime->rpc_response_retry_blocked = 0U;\n  result = wl_rpc_server_response_acquire(runtime->rpc_server, &response);\n  if (result == WL_RPC_ERR_NOT_FOUND) return WL_RPC_OK;\n  if (result != WL_RPC_OK) return result;\n  out_result->response.message_id = response.identity.response_message_id;\n  out_result->response.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  out_result->response.detail.rpc.operation_id = response.identity.operation_id;\n  out_result->response.detail.rpc.application_result = response.application_status;\n  out_result->response.detail.rpc.payload_length = response.response_length;\n  out_result->response.detail.rpc.server_response = response;\n  switch (response.identity.response_message_id) {{\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        let request_macro = upper_snake(&service.request_name);
        let response_macro = upper_snake(&service.response_name);
        let reliable = match service.response_delivery {
            DeliveryPolicy::Unreliable => "0U",
            DeliveryPolicy::Reliable => "1U",
        };
        writeln!(
            output,
            "    case {response_macro}_MESSAGE_ID:\n      if (response.identity.request_message_id != {request_macro}_MESSAGE_ID) {{\n        result = WL_RPC_ERR_RESPONSE_MISMATCH;\n        break;\n      }}\n      reliable_response = {reliable};\n      break;"
        )
        .unwrap();
    }
    write!(
        output,
        "    default:\n      result = WL_RPC_ERR_RESPONSE_MISMATCH;\n      break;\n  }}\n  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  if (reliable_response != 0U) {{\n    out_result->response.detail.rpc.core_result = wl_send_reliable(ctx, response.identity.response_message_id, response.response_data, response.response_length, now_ms, &out_result->response.detail.rpc.handle);\n  }} else {{\n    out_result->response.detail.rpc.core_result = wl_send_unreliable(ctx, response.identity.response_message_id, response.response_data, response.response_length);\n  }}\n  if (out_result->response.detail.rpc.core_result != WL_OK) {{\n    result = wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    if (result != WL_RPC_OK) return result;\n    out_result->response.domain = {prefix}_RUNTIME_CORE_ERROR;\n    out_result->responses_deferred = 1U;\n    runtime->rpc_response_retry_generation = wl_link_resource_generation(ctx);\n    runtime->rpc_response_retry_blocked = 1U;\n    return WL_RPC_OK;\n  }}\n  if (reliable_response != 0U) {{\n    result = wl_rpc_server_response_submitted(runtime->rpc_server, &response, out_result->response.detail.rpc.handle);\n  }} else {{\n    result = wl_rpc_server_response_sent(runtime->rpc_server, &response);\n  }}\n"
    )
    .unwrap();
    write!(
        output,
        "  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  out_result->response.domain = {prefix}_RUNTIME_OK;\n  out_result->responses_submitted = 1U;\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_get_deadline_hint(const {module}_runtime_t *runtime, wl_time_ms_t now_ms, wl_rpc_deadline_hint_t *out_hint) {{\n  wl_rpc_deadline_hint_t component = {{WL_RPC_NO_DEADLINE_MS}};\n  wl_rpc_err_t result;\n  uint32_t nearest = WL_RPC_NO_DEADLINE_MS;\n  if (out_hint != NULL) out_hint->next_deadline_ms = WL_RPC_NO_DEADLINE_MS;\n  if (runtime == NULL || out_hint == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_get_deadline_hint(runtime->rpc_client, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = {server_deadline}(runtime->rpc_server, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  out_hint->next_deadline_ms = nearest;\n  return WL_RPC_OK;\n}}\n\n"
    )
    .unwrap();
    write!(output, "wl_readiness_t {module}_runtime_readiness(const wl_ctx_t *ctx, const {module}_runtime_t *runtime, wl_time_ms_t now_ms) {{\n  wl_readiness_t value = wl_readiness_idle();\n  wl_rpc_deadline_hint_t timer;\n  uint16_t message_id = 0U;\n  if (runtime == NULL || ctx == NULL) return value;\n  if ({module}_runtime_get_deadline_hint(runtime, now_ms, &timer) == WL_RPC_OK)\n    value = wl_readiness_at(timer.next_deadline_ms);\n  if (runtime->rpc_retiring_tx != 0U) {{\n    if (wl_tx_can_take(ctx, runtime->rpc_retiring_tx)) value.runnable = 1U;\n    else value.blocked = 1U;\n  }}\n  if (runtime->rpc_server != NULL && wl_rpc_server_response_peek(runtime->rpc_server, &message_id) == WL_RPC_OK) {{\n    wl_delivery_t delivery = WL_DELIVERY_UNRELIABLE;\n    switch (message_id) {{\n").unwrap();
    for service in &profile.rpc_services {
        let response_macro = upper_snake(&service.response_name);
        let delivery = delivery_value(service.response_delivery);
        writeln!(
            output,
            "      case {response_macro}_MESSAGE_ID: delivery = {delivery}; break;"
        )
        .unwrap();
    }
    output.push_str("      default: break;\n    }\n    if (wl_tx_can_submit(ctx, delivery) && (!runtime->rpc_response_retry_blocked ||\n        runtime->rpc_response_retry_generation != wl_link_resource_generation(ctx))) value.runnable = 1U;\n    else value.blocked = 1U;\n  }\n  return value;\n}\n\n");
}

pub(super) fn delivery_event(delivery: DeliveryPolicy) -> &'static str {
    match delivery {
        DeliveryPolicy::Unreliable => "WL_EVT_UNRELIABLE_RX",
        DeliveryPolicy::Reliable => "WL_EVT_RELIABLE_RX",
    }
}

pub(super) fn delivery_value(delivery: DeliveryPolicy) -> &'static str {
    match delivery {
        DeliveryPolicy::Unreliable => "WL_DELIVERY_UNRELIABLE",
        DeliveryPolicy::Reliable => "WL_DELIVERY_RELIABLE",
    }
}

pub(super) fn emit_rpc_request_case(
    output: &mut String,
    module: &str,
    prefix: &str,
    service: &RpcService,
) {
    if service.is_managed() {
        output.push_str(&crate::managed_rpc_codegen::request_case(module, service));
        return;
    }
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let request_macro = upper_snake(&service.request_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.as_ref().unwrap().name);
    let expected_event = delivery_event(service.request_delivery);
    let delivery = delivery_value(service.request_delivery);
    let now_ms = "now_ms";
    let replay = service.guarantee == crate::profile_semantic::RpcGuarantee::Replay;
    let seed = format!(".request_fingerprint = {module}_rpc_fingerprint_seed");
    let begin = if replay {
        "      result.detail.rpc.rpc_result = wl_rpc_server_begin(runtime->rpc_server, &identity, now_ms, &result.detail.rpc.rpc_disposition, &server_request, &replay);\n"
    } else {
        "      result.detail.rpc.rpc_result = wl_rpc_server_begin_correlate(runtime->rpc_server, &identity, now_ms, &server_request);\n      result.detail.rpc.rpc_disposition = WL_RPC_SERVER_NEW;\n"
    };
    let peer_observe = format!(
        "      if (event->peer_session_id != 0U && runtime->rpc_peer.session_id != event->peer_session_id) {{\n        wl_rpc_peer_observation_t observation = {{0}};\n        result.detail.rpc.rpc_result = {module}_runtime_peer_observe(ctx, runtime, event->peer_session_id, &observation);\n        if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n        }}\n        if (observation.changed != 0U) result.detail.rpc.peer_changed = 1U;\n      }}\n"
    );
    let fingerprint = if replay {
        include_str!("../mapped_rpc_fingerprint.c.in")
    } else {
        "      result.detail.rpc.payload_length = event->payload_len;\n"
    };
    let mut values = vec![
        ("MODULE", module),
        ("PREFIX", prefix),
        ("REQUEST_MACRO", &request_macro),
        ("RESPONSE_MACRO", &response_macro),
        ("SERVICE_NAME", &service_name),
        ("REQUEST", &request),
        ("OPERATION_FIELD", &operation_field),
        ("EXPECTED_EVENT", expected_event),
        ("DELIVERY", delivery),
        ("NOW_MS", now_ms),
    ];
    let fingerprint = crate::template::render(fingerprint, &values);
    let dispositions = crate::template::render(
        if replay {
            include_str!("../mapped_rpc_replay.c.in")
        } else {
            ""
        },
        &values,
    );
    values.extend([
        ("FINGERPRINT_INIT", if replay { &seed } else { "0" }),
        ("FINGERPRINT", &fingerprint), ("PEER_OBSERVE", &peer_observe), ("BEGIN", begin),
        ("REPLAY_DISPOSITIONS", &dispositions),
        ("REPLAY_VARIABLES", if replay { "      wl_rpc_server_response_t replay = {0};\n      size_t canonical_length = 0U;\n" } else { "" }),
    ]);
    output.push_str(&crate::template::render(
        include_str!("../mapped_rpc_request.c.in"),
        &values,
    ));
}

pub(super) fn emit_rpc_response_case(
    output: &mut String,
    module: &str,
    prefix: &str,
    service: &RpcService,
) {
    if service.is_managed() {
        output.push_str(&crate::managed_rpc_codegen::response_case(module, service));
        return;
    }
    let service_name = c_identifier(&service.name);
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.response_operation_id.as_ref().unwrap().name);
    let status_field = c_identifier(&service.response_status.as_ref().unwrap().name);
    let expected_event = delivery_event(service.response_delivery);
    let request_macro = upper_snake(&service.request_name);
    write!(output, r#"    case {response_macro}_MESSAGE_ID: {{
      wl_rpc_client_response_t authority;
      uint16_t result_index;
      {response}_value_t *decoded = &runtime->rpc_mapped_scratch.{service_name};
      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;
      if (event->type != {expected_event}) {{
        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;
        break;
      }}
      if (runtime->rpc_client == NULL) {{
        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;
        break;
      }}
      /* Mapped correlation fields are in the schema body: decode once into
       * owned scratch, validate authority, then copy into the selected slot. */
      result.detail.rpc.codec_status = {response}_value_decode(event->payload, event->payload_len, decoded);
      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{
        result.domain = {prefix}_RUNTIME_CODEC_ERROR;
        break;
      }}
      if (!decoded->has_{operation_field} || decoded->{operation_field} == 0U || !decoded->has_{status_field}) {{
        result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;
        result.domain = {prefix}_RUNTIME_RPC_ERROR;
        break;
      }}
      result.detail.rpc.operation_id = decoded->{operation_field};
      result.detail.rpc.application_result = (int32_t)decoded->{status_field};
      result.detail.rpc.payload_length = event->payload_len;
      result.detail.rpc.rpc_result = wl_rpc_client_response_prepare(runtime->rpc_client,
          {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, decoded->{operation_field}, now_ms,
          &authority, &result_index);
      if (result.detail.rpc.rpc_result != WL_RPC_OK) {{
        result.domain = result.detail.rpc.rpc_result == WL_RPC_ERR_NOT_FOUND || result.detail.rpc.rpc_result == WL_RPC_ERR_INVALID_STATE ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;
        break;
      }}
      if (runtime->rpc_results == NULL || result_index >= runtime->rpc_result_count) {{
        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;
        break;
      }}
      if (result.detail.rpc.application_result == 0)
        runtime->rpc_results[result_index].{service_name} = *decoded;
      result.detail.rpc.rpc_result = wl_rpc_client_response_commit(runtime->rpc_client,
          &authority, result.detail.rpc.application_result, now_ms);
      result.domain = result.detail.rpc.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;
      break;
    }}
"#).unwrap();
}

pub(super) fn emit_rpc_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    codec_module: &str,
    codec_prefix: &str,
    service: &RpcService,
) {
    if service.is_managed() {
        output.push_str(&crate::managed_rpc_codegen::implementation(module, service));
        return;
    }
    emit_rpc_client_implementation(output, module, prefix, codec_module, codec_prefix, service);
    emit_rpc_server_implementation(output, module, prefix, service);
}

pub(super) fn emit_rpc_client_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    codec_module: &str,
    codec_prefix: &str,
    service: &RpcService,
) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let request_macro = upper_snake(&service.request_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.as_ref().unwrap().name);
    let delivery = delivery_value(service.request_delivery);
    let transition = match service.request_delivery {
        DeliveryPolicy::Unreliable => {
            "wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)"
        }
        DeliveryPolicy::Reliable => {
            "wl_rpc_client_bind_tx(runtime->rpc_client, operation_id, result.detail.rpc.handle)"
        }
    };
    write!(
        output,
        "static {module}_runtime_result_t {module}_{service_name}_client_finish_start({module}_runtime_t *runtime, uint32_t operation_id, {codec_module}_send_result_t sent) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {request_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  result.detail.rpc.operation_id = operation_id;\n  result.detail.rpc.codec_status = sent.codec_status;\n  result.detail.rpc.core_result = sent.core_result;\n  result.detail.rpc.handle = sent.handle;\n  result.detail.rpc.payload_length = sent.payload_length;\n  if (sent.domain == {codec_prefix}_SEND_CODEC_ERROR || sent.domain == {codec_prefix}_SEND_CORE_ERROR) {{\n    const int32_t link_result = sent.domain == {codec_prefix}_SEND_CODEC_ERROR ? WL_ERR_CORRUPT_PAYLOAD : sent.core_result;\n    result.detail.rpc.rpc_result = wl_rpc_client_link_failed(runtime->rpc_client, operation_id, link_result);\n    if (result.detail.rpc.rpc_result == WL_RPC_OK)\n      result.detail.rpc.rpc_result = wl_rpc_client_release(runtime->rpc_client, operation_id);\n    if (result.detail.rpc.rpc_result == WL_RPC_OK) result.detail.rpc.operation_id = 0U;\n    result.domain = sent.domain == {codec_prefix}_SEND_CODEC_ERROR ? {prefix}_RUNTIME_CODEC_ERROR : {prefix}_RUNTIME_CORE_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = {transition};\n  result.domain = result.detail.rpc.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n  return result;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_start(wl_ctx_t *ctx, {module}_runtime_t *runtime, const {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  {codec_module}_send_result_t sent;\n  {request}_t *encoded_request;\n  uint32_t operation_id = 0U;\n  result.message_id = {request_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  if (ctx == NULL || runtime == NULL || runtime->rpc_client == NULL || request == NULL) return result;\n  if (runtime->rpc_encode_scratch == NULL) {{\n    result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n    return result;\n  }}\n  encoded_request = &runtime->rpc_encode_scratch->{service_name}_request;\n  if ((const void *)request == (const void *)encoded_request) return result;\n  if (request->has_{operation_field} && request->{operation_field} != 0U) {{\n    operation_id = request->{operation_field};\n    result.detail.rpc.rpc_result = wl_rpc_client_begin_with_id(runtime->rpc_client, operation_id, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms);\n  }} else {{\n    result.detail.rpc.rpc_result = wl_rpc_client_begin(runtime->rpc_client, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms, &operation_id);\n  }}\n  result.detail.rpc.operation_id = operation_id;\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  *encoded_request = *request;\n  encoded_request->has_{operation_field} = true;\n  encoded_request->{operation_field} = operation_id;\n  sent = {codec_module}_{request}_send(ctx, encoded_request, {delivery}, now_ms);\n  return {module}_{service_name}_client_finish_start(runtime, operation_id, sent);\n}}\n\n"
    )
    .unwrap();
    write!(
        output,
        "wl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client) {{\n  wl_rpc_err_t result;\n  if (out_client != NULL) memset(out_client, 0, sizeof(*out_client));\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U || out_client == NULL) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, out_client);\n  if (result != WL_RPC_OK) return result;\n  if (out_client->request_message_id != {request_macro}_MESSAGE_ID || out_client->response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id) {{\n  wl_rpc_client_result_t client = {{0}};\n  wl_rpc_err_t result;\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, &client);\n  if (result != WL_RPC_OK) return result;\n  if (client.request_message_id != {request_macro}_MESSAGE_ID || client.response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return wl_rpc_client_release(runtime->rpc_client, operation_id);\n}}\n\n"
    )
    .unwrap();
}

pub(super) fn emit_rpc_server_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    service: &RpcService,
) {
    let service_name = c_identifier(&service.name);
    let request_macro = upper_snake(&service.request_name);
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.response_operation_id.as_ref().unwrap().name);
    let status_field = c_identifier(&service.response_status.as_ref().unwrap().name);
    write!(
        output,
        "static {module}_runtime_result_t {module}_{service_name}_server_finish({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, const {response}_t *response, wl_time_ms_t now_ms, bool reject) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  wl_rpc_server_response_buffer_t buffer = {{0}};\n  wl_rpc_server_response_t cached = {{0}};\n  {response}_t *encoded_response;\n  size_t encoded_length = 0U;\n  result.message_id = {response_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  result.detail.rpc.application_result = application_status;\n  if (runtime == NULL || runtime->rpc_server == NULL || server_request == NULL || server_request->generation == 0U || server_request->identity.operation_id == 0U || server_request->identity.request_message_id != {request_macro}_MESSAGE_ID || server_request->identity.response_message_id != {response_macro}_MESSAGE_ID || response == NULL) return result;\n  result.detail.rpc.operation_id = server_request->identity.operation_id;\n  result.detail.rpc.server_request = *server_request;\n  if (runtime->rpc_encode_scratch == NULL) {{\n    result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n    return result;\n  }}\n  encoded_response = &runtime->rpc_encode_scratch->{service_name}_response;\n  if ((const void *)response == (const void *)encoded_response) return result;\n  if (reject && application_status == 0) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_ARG;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = wl_rpc_server_response_prepare(runtime->rpc_server, server_request, &buffer);\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  *encoded_response = *response;\n  encoded_response->has_{operation_field} = true;\n  encoded_response->{operation_field} = server_request->identity.operation_id;\n  encoded_response->has_{status_field} = true;\n  encoded_response->{status_field} = application_status;\n  result.detail.rpc.codec_status = {response}_encode(encoded_response, buffer.data, buffer.capacity, &encoded_length);\n  result.detail.rpc.payload_length = encoded_length;\n  if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = wl_rpc_server_response_commit(runtime->rpc_server, &buffer, application_status, encoded_length, now_ms, &cached);\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.server_response = cached;\n  result.detail.rpc.application_result = cached.application_status;\n  result.detail.rpc.payload_length = cached.response_length;\n  result.detail.rpc.core_result = WL_OK;\n  result.domain = {prefix}_RUNTIME_OK;\n  return result;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_complete({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, const {response}_t *response, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(runtime, server_request, 0, response, now_ms, false);\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_reject({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, const {response}_t *response, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(runtime, server_request, application_status, response, now_ms, true);\n}}\n"
    )
    .unwrap();
}
