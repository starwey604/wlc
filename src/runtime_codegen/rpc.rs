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
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* The decoded request and its borrowed fields are valid only for the\n * callback. Copy server_request for asynchronous completion. Its generation\n * prevents a late completion from targeting a reused request identity. A\n * nonzero return abandons this exact pending operation. */\ntypedef int32_t (*{module}_{service_name}_rpc_request_handler_fn)(void *user_data, const {request}_t *request, const wl_rpc_server_request_t *server_request, wl_delivery_t delivery);\ntypedef struct {{\n  {request}_t *request_scratch;\n  {response}_t *response_scratch;\n  {module}_{service_name}_rpc_request_handler_fn request_handler;\n  void *user_data;\n}} {module}_{service_name}_rpc_t;\n\n"
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
        "/* Allocates, encodes, and submits atomically from the caller's view. A\n * present nonzero request operation ID is used exactly, allowing an explicit\n * retry to address the server's bounded replay cache; absent or zero selects an\n * automatically allocated ID. The const request is copied into runtime-owned\n * encode scratch before operation ID injection. A local encode/submit failure\n * releases the allocated RPC slot and returns operation_id zero. */\n{module}_runtime_result_t {module}_{service_name}_client_start(wl_ctx_t *ctx, {module}_runtime_t *runtime, const {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms);\n/* Nonblocking inspection returns generic metadata for this service. */\nwl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client);\n/* Decode a retained response previously returned by client_inspect(). Borrowed\n * response fields remain valid only until client_release(). */\n{module}_runtime_result_t {module}_{service_name}_client_decode(const wl_rpc_client_result_t *client, {response}_t *response);\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id);\n\n/* server_request is copied from the request callback and uniquely scopes this\n * execution generation. Completion copies the const response into runtime-owned\n * encode scratch before injecting operation ID/status. runtime_service() later\n * submits the cached response bytes. */\n{module}_runtime_result_t {module}_{service_name}_server_complete({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, const {response}_t *response, wl_time_ms_t now_ms);\n{module}_runtime_result_t {module}_{service_name}_server_reject({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, const {response}_t *response, wl_time_ms_t now_ms);\n\n"
    )
    .unwrap();
}

pub(super) fn emit_rpc_runtime_progress_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    profile: &BindingProfileModel,
) {
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
        "wl_rpc_err_t {module}_runtime_poll({module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_poll_result_t *out_result) {{\n  wl_rpc_err_t result;\n  wl_rpc_server_expiry_t server_expiry = {{0}};\n  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_poll(runtime->rpc_client, now_ms, &out_result->client_timed_out);\n    if (result != WL_RPC_OK) return result;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = wl_rpc_server_expired_acquire(runtime->rpc_server, now_ms, &out_result->server_expired_request);\n    if (result == WL_RPC_OK) out_result->server_pending_expired = 1U;\n    else if (result != WL_RPC_ERR_NOT_FOUND) return result;\n    result = wl_rpc_server_poll(runtime->rpc_server, now_ms, &server_expiry);\n    if (result != WL_RPC_OK) return result;\n    out_result->server_cache_expired = server_expiry.cache_expired;\n  }}\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_service(wl_ctx_t *ctx, {module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_service_result_t *out_result) {{\n  wl_rpc_server_response_t response = {{0}};\n  wl_rpc_err_t result;\n  uint8_t reliable_response = 0U;\n  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (ctx == NULL || runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  out_result->response = {module}_runtime_result(NULL);\n  if (runtime->rpc_retiring_tx != 0U) {{\n    wl_tx_result_t ignored;\n    int retired = wl_tx_take(ctx, runtime->rpc_retiring_tx, &ignored);\n    if (retired == WL_OK || retired == WL_ERR_NOT_FOUND) runtime->rpc_retiring_tx = 0U;\n  }}\n  result = {module}_runtime_poll(runtime, now_ms, &out_result->deadlines);\n  if (result != WL_RPC_OK) return result;\n  if (runtime->rpc_server == NULL) return WL_RPC_OK;\n  result = wl_rpc_server_response_acquire(runtime->rpc_server, &response);\n  if (result == WL_RPC_ERR_NOT_FOUND) return WL_RPC_OK;\n  if (result != WL_RPC_OK) return result;\n  out_result->response.message_id = response.identity.response_message_id;\n  out_result->response.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  out_result->response.detail.rpc.operation_id = response.identity.operation_id;\n  out_result->response.detail.rpc.application_result = response.application_status;\n  out_result->response.detail.rpc.payload_length = response.response_length;\n  out_result->response.detail.rpc.server_response = response;\n  switch (response.identity.response_message_id) {{\n"
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
        "    default:\n      result = WL_RPC_ERR_RESPONSE_MISMATCH;\n      break;\n  }}\n  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  if (reliable_response != 0U) {{\n    out_result->response.detail.rpc.core_result = wl_send_reliable(ctx, response.identity.response_message_id, response.response_data, response.response_length, now_ms, &out_result->response.detail.rpc.handle);\n  }} else {{\n    out_result->response.detail.rpc.core_result = wl_send_unreliable(ctx, response.identity.response_message_id, response.response_data, response.response_length);\n  }}\n  if (out_result->response.detail.rpc.core_result != WL_OK) {{\n    result = wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    if (result != WL_RPC_OK) return result;\n    out_result->response.domain = {prefix}_RUNTIME_CORE_ERROR;\n    out_result->responses_deferred = 1U;\n    return WL_RPC_OK;\n  }}\n  if (reliable_response != 0U) {{\n    result = wl_rpc_server_response_submitted(runtime->rpc_server, &response, out_result->response.detail.rpc.handle);\n  }} else {{\n    result = wl_rpc_server_response_sent(runtime->rpc_server, &response);\n  }}\n"
    )
    .unwrap();
    write!(
        output,
        "  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  out_result->response.domain = {prefix}_RUNTIME_OK;\n  out_result->responses_submitted = 1U;\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_get_deadline_hint(const {module}_runtime_t *runtime, wl_time_ms_t now_ms, wl_rpc_deadline_hint_t *out_hint) {{\n  wl_rpc_deadline_hint_t component = {{WL_RPC_NO_DEADLINE_MS}};\n  wl_rpc_err_t result;\n  uint32_t nearest = WL_RPC_NO_DEADLINE_MS;\n  if (out_hint != NULL) out_hint->next_deadline_ms = WL_RPC_NO_DEADLINE_MS;\n  if (runtime == NULL || out_hint == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_get_deadline_hint(runtime->rpc_client, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = wl_rpc_server_get_deadline_hint(runtime->rpc_server, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  out_hint->next_deadline_ms = nearest;\n  return WL_RPC_OK;\n}}\n\n"
    )
    .unwrap();
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
    write!(
        output,
        "    case {request_macro}_MESSAGE_ID: {{\n      wl_rpc_request_identity_t identity = {{.request_fingerprint = {module}_rpc_fingerprint_seed}};\n      wl_rpc_server_request_t server_request = {{0}};\n      wl_rpc_server_response_t replay = {{0}};\n      size_t canonical_length = 0U;\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_server == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.request_scratch == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.detail.rpc.codec_status = {request}_decode(event->payload, event->payload_len, runtime->{service_name}.request_scratch);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.request_scratch->has_{operation_field} || runtime->{service_name}.request_scratch->{operation_field} == 0U) {{\n        result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_ARG;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.detail.rpc.operation_id = runtime->{service_name}.request_scratch->{operation_field};\n      result.detail.rpc.codec_status = {request}_wlc_detail_fingerprint(runtime->{service_name}.request_scratch, &identity.request_fingerprint, &canonical_length);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.detail.rpc.payload_length = canonical_length;\n      identity.operation_id = result.detail.rpc.operation_id;\n      identity.request_message_id = {request_macro}_MESSAGE_ID;\n      identity.response_message_id = {response_macro}_MESSAGE_ID;\n      identity.peer_session_id = event->peer_session_id;\n      result.detail.rpc.rpc_result = wl_rpc_server_begin(runtime->rpc_server, &identity, {now_ms}, &result.detail.rpc.rpc_disposition, &server_request, &replay);\n      if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      switch (result.detail.rpc.rpc_disposition) {{\n        case WL_RPC_SERVER_NEW:\n          result.detail.rpc.server_request = server_request;\n          if (runtime->{service_name}.request_handler == NULL) {{\n            result.detail.rpc.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, &server_request);\n            result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n            break;\n          }}\n          result.detail.rpc.application_result = runtime->{service_name}.request_handler(runtime->{service_name}.user_data, runtime->{service_name}.request_scratch, &server_request, {delivery});\n          if (result.detail.rpc.application_result != 0) {{\n            result.detail.rpc.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, &server_request);\n            result.domain = {prefix}_RUNTIME_APPLICATION_ERROR;\n          }} else {{\n            result.domain = {prefix}_RUNTIME_OK;\n          }}\n          break;\n        case WL_RPC_SERVER_PENDING_DUPLICATE:\n          result.domain = {prefix}_RUNTIME_OK;\n          break;\n        case WL_RPC_SERVER_REPLAY:\n          result.detail.rpc.server_response = replay;\n          result.detail.rpc.application_result = replay.application_status;\n          result.detail.rpc.payload_length = replay.response_length;\n          result.detail.rpc.core_result = WL_OK;\n          result.domain = {prefix}_RUNTIME_OK;\n          break;\n        case WL_RPC_SERVER_CONFLICT:\n          result.detail.rpc.rpc_result = WL_RPC_ERR_OPERATION_CONFLICT;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n        default:\n          result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_STATE;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n      }}\n      break;\n    }}\n"
    )
    .unwrap();
    let peer_observe_marker = format!(
        "      if (runtime->rpc_server == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n"
    );
    let peer_observe_offset = output
        .rfind(&peer_observe_marker)
        .expect("generated RPC request case has server guard")
        + peer_observe_marker.len();
    let peer_observe = format!(
        "      if (event->peer_session_id != 0U && runtime->rpc_peer.session_id != event->peer_session_id) {{\n        wl_rpc_peer_observation_t observation = {{0}};\n        result.detail.rpc.rpc_result = {module}_runtime_peer_observe(ctx, runtime, event->peer_session_id, &observation);\n        if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n        }}\n        if (observation.changed != 0U) result.detail.rpc.peer_changed = 1U;\n      }}\n"
    );
    output.insert_str(peer_observe_offset, &peer_observe);
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
    write!(
        output,
        "    case {response_macro}_MESSAGE_ID: {{\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_client == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.response_scratch == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.detail.rpc.codec_status = {response}_decode(event->payload, event->payload_len, runtime->{service_name}.response_scratch);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.response_scratch->has_{operation_field} || runtime->{service_name}.response_scratch->{operation_field} == 0U || !runtime->{service_name}.response_scratch->has_{status_field}) {{\n        result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.detail.rpc.operation_id = runtime->{service_name}.response_scratch->{operation_field};\n      result.detail.rpc.application_result = (int32_t)runtime->{service_name}.response_scratch->{status_field};\n      result.detail.rpc.payload_length = event->payload_len;\n      result.detail.rpc.rpc_result = wl_rpc_client_on_response(runtime->rpc_client, {response_macro}_MESSAGE_ID, result.detail.rpc.operation_id, result.detail.rpc.application_result, event->payload, event->payload_len);\n      result.domain = result.detail.rpc.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n      break;\n    }}\n"
    )
    .unwrap();
    let _ = module;
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
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.as_ref().unwrap().name);
    let response_operation_field =
        c_identifier(&service.response_operation_id.as_ref().unwrap().name);
    let status_field = c_identifier(&service.response_status.as_ref().unwrap().name);
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
        "wl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client) {{\n  wl_rpc_err_t result;\n  if (out_client != NULL) memset(out_client, 0, sizeof(*out_client));\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U || out_client == NULL) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, out_client);\n  if (result != WL_RPC_OK) return result;\n  if (out_client->request_message_id != {request_macro}_MESSAGE_ID || out_client->response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return WL_RPC_OK;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_decode(const wl_rpc_client_result_t *client, {response}_t *response) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {response_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  if (client != NULL) {{\n    result.detail.rpc.operation_id = client->operation_id;\n    result.detail.rpc.handle = client->tx_handle;\n    result.detail.rpc.core_result = client->link_result;\n    result.detail.rpc.application_result = client->application_status;\n    result.detail.rpc.payload_length = client->response_length;\n  }}\n  if (client == NULL || response == NULL || client->operation_id == 0U) return result;\n  {response}_clear(response);\n  if (client->request_message_id != {request_macro}_MESSAGE_ID || client->response_message_id != {response_macro}_MESSAGE_ID) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  if ((client->state != WL_RPC_CLIENT_COMPLETED && client->state != WL_RPC_CLIENT_APPLICATION_ERROR) || client->response_data == NULL || client->response_length == 0U) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_STATE;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.codec_status = {response}_decode(client->response_data, client->response_length, response);\n  if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  if (!response->has_{response_operation_field} || response->{response_operation_field} != client->operation_id || !response->has_{status_field} || (int32_t)response->{status_field} != client->application_status) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.domain = {prefix}_RUNTIME_OK;\n  return result;\n}}\n\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id) {{\n  wl_rpc_client_result_t client = {{0}};\n  wl_rpc_err_t result;\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, &client);\n  if (result != WL_RPC_OK) return result;\n  if (client.request_message_id != {request_macro}_MESSAGE_ID || client.response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return wl_rpc_client_release(runtime->rpc_client, operation_id);\n}}\n\n"
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
