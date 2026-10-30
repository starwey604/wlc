// SPDX-License-Identifier: Apache-2.0
use super::assembly::emit_assembly_source;
use super::pump::emit_pump_implementation;
use super::retained::{emit_retained_case, emit_retained_implementation};
use super::rpc::{
    emit_rpc_implementation, emit_rpc_request_case, emit_rpc_response_case,
    emit_rpc_runtime_progress_implementation,
};
use crate::codegen::{c_identifier, type_name, upper_snake};
use crate::profile_semantic::{BindingProfileModel, RpcGuarantee, RpcService};
use crate::rpc_plan::RpcEndpointPlan;
use std::collections::HashSet;
use std::fmt::Write;

pub(super) fn emit_source(
    schema: &crate::semantic::SemanticModel,
    rpc_plan: &RpcEndpointPlan,
    profile: &BindingProfileModel,
    codec_module: &str,
    module: &str,
) -> String {
    let prefix = upper_snake(module);
    let mut output = format!(
        "#include \"{module}_runtime.h\"\n\n#include <string.h>\n\nstatic {module}_runtime_result_t {module}_runtime_result(const wl_event_t *event) {{\n  {module}_runtime_result_t result = {{0}};\n  result.domain = {prefix}_RUNTIME_INVALID_ARGUMENT;\n  if (event != NULL) {{\n    result.message_id = event->message_id;\n    result.event_type = event->type;\n  }}\n  return result;\n}}\n\n"
    );
    writeln!(output, "_Static_assert({prefix}_SCHEMA_IDENTITY == UINT64_C(0x{:016X}) && {prefix}_BINDING_PROFILE_IDENTITY == UINT64_C(0x{:016X}), \"Runtime source/header profile mismatch; regenerate the matched artifacts.\");\n", crate::schema_identity(schema), crate::binding_profile_identity(profile)).unwrap();
    emit_result_str_implementation(&mut output, module, &prefix);
    if profile.rpc_services.iter().any(RpcService::is_managed) {
        output.push_str(&crate::managed_rpc_codegen::private_source(
            schema, module, profile,
        ));
        output.push_str(&crate::managed_rpc_codegen::helpers(module, profile));
    }
    if !profile.rpc_services.is_empty() {
        // These declarations are private to the matched codec/runtime pair.
        // Do not put trusted entry points in application-facing headers.
        let seed = b"wlc.rpc.canonical-request.v1\xff"
            .iter()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x00000100000001b3)
            });
        if profile.has_rpc_server() && profile.has_replay() {
            writeln!(
                output,
                "static const uint64_t {module}_rpc_fingerprint_seed = UINT64_C({seed:#018x});"
            )
            .unwrap();
        }
        let mut requests = HashSet::new();
        for service in &profile.rpc_services {
            let name = type_name(&service.request_name);
            if requests.insert(name.clone()) {
                let request_prefix = upper_snake(&service.request_name);
                if profile.has_rpc_server()
                    && profile.rpc_services.iter().any(|s| {
                        s.request_name == service.request_name
                            && s.guarantee == RpcGuarantee::Replay
                    })
                {
                    writeln!(output, "wl_codec_status_t {name}_wlc_detail_fingerprint(const {name}_t *, uint64_t *, size_t *);").unwrap();
                }
                writeln!(output, "#if {request_prefix}_HAS_VALUE\nvoid {name}_wlc_detail_value_copy(const {name}_t *, {name}_value_t *);\n#endif\n").unwrap();
            }
        }
        write!(
            output,
            "typedef struct {{ wl_ctx_t *link; {module}_runtime_t *runtime; }} {module}_peer_cancel_context_t;\nstatic void {module}_runtime_cancel_peer_tx(void *context, wl_tx_handle_t handle) {{\n  {module}_peer_cancel_context_t *cancel = context;\n  wl_tx_result_t ignored;\n  if (cancel == NULL) return;\n  (void)wl_tx_cancel(cancel->link, handle);\n  /* A cancelled transaction need not emit a terminal event. Take it now, or\n   * retain just its handle until the adapter releases physical TX storage. */\n  if (wl_tx_take(cancel->link, handle, &ignored) == WL_ERR_INVALID_STATE)\n    cancel->runtime->rpc_retiring_tx = handle;\n}}\n\n"
        )
        .unwrap();
    }
    emit_assembly_source(&mut output, rpc_plan, profile, module);
    if !profile.rpc_services.is_empty() {
        emit_rpc_runtime_progress_implementation(&mut output, module, &prefix, profile);
        // RPC dispatch includes request admission, response matching and cache
        // handling. Keep that body out of the pump's hot loop when using LTO.
        output.push_str("/* Keep RPC routing separate from the pump under LTO. */\n#if defined(_MSC_VER)\n__declspec(noinline)\n#elif defined(__GNUC__) || defined(__clang__)\n__attribute__((noinline))\n#endif\n");
    }
    write!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime, wl_time_ms_t now_ms) {{\n  {module}_runtime_result_t result = {module}_runtime_result(event);\n  if (event == NULL) return result;\n"
    )
    .unwrap();
    output.push_str("  (void)now_ms;\n");
    if profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n"
        )
        .unwrap();
    } else {
        write!(
            output,
            "  if (event->type == WL_EVT_TX_SUCCESS || event->type == WL_EVT_TX_TIMEOUT || event->type == WL_EVT_TX_FAILED) {{\n    wl_tx_result_t tx_result = {{0}};\n    if (runtime == NULL || ctx == NULL) {{\n      result.domain = {prefix}_RUNTIME_NON_RX;\n      return result;\n    }}\n    result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n    result.detail.rpc.handle = event->handle;\n#if {prefix}_RUNTIME_HAS_MANAGED_RPC\n    if (runtime->rpc_async != NULL && wl_rpc_async_retire_tx(runtime->rpc_async, event->handle)) {{\n      result.detail.rpc.core_result = wl_tx_take(ctx, event->handle, &tx_result);\n      result.event_consumed = result.detail.rpc.core_result == WL_OK ? 1U : 0U;\n      result.domain = result.detail.rpc.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n      return result;\n    }}\n#endif\n    if (runtime->rpc_server != NULL) {{\n      result.detail.rpc.rpc_result = wl_rpc_server_on_tx_event(runtime->rpc_server, event);\n      if (result.detail.rpc.rpc_result == WL_RPC_OK) {{\n        result.detail.rpc.core_result = wl_tx_take(ctx, event->handle, &tx_result);\n        result.event_consumed = result.detail.rpc.core_result == WL_OK ? 1U : 0U;\n        result.domain = result.detail.rpc.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n        return result;\n      }}\n      if (result.detail.rpc.rpc_result != WL_RPC_ERR_NOT_FOUND) {{\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        return result;\n      }}\n    }}\n    if (runtime->rpc_client != NULL) {{\n      result.detail.rpc.rpc_result = wl_rpc_client_on_tx_event(runtime->rpc_client, event);\n      if (result.detail.rpc.rpc_result == WL_RPC_OK) {{\n        result.detail.rpc.core_result = wl_tx_take(ctx, event->handle, &tx_result);\n        result.event_consumed = result.detail.rpc.core_result == WL_OK ? 1U : 0U;\n        result.domain = result.detail.rpc.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n      }} else if (result.detail.rpc.rpc_result == WL_RPC_ERR_NOT_FOUND) result.domain = {prefix}_RUNTIME_NON_RX;\n      else result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    }} else {{\n      result.domain = {prefix}_RUNTIME_NON_RX;\n    }}\n    return result;\n  }}\n  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n"
        )
        .unwrap();
    }
    output.push_str("  if (ctx == NULL) return result;\n  if (runtime == NULL) goto release_event;\n\n  switch (event->message_id) {\n");
    for route in &profile.retained_routes {
        emit_retained_case(&mut output, module, &prefix, route);
    }
    for route in &profile.direct_routes {
        super::direct::emit_case(&mut output, &prefix, route);
    }
    for service in &profile.rpc_services {
        if profile.has_rpc_server() {
            emit_rpc_request_case(&mut output, module, &prefix, service);
        } else {
            emit_disabled_rpc_case(
                &mut output,
                &prefix,
                service.request_id,
                service.request_delivery,
            );
        }
        if profile.has_rpc_client() {
            emit_rpc_response_case(&mut output, module, &prefix, service);
        } else {
            emit_disabled_rpc_case(
                &mut output,
                &prefix,
                service.response_id,
                service.response_delivery,
            );
        }
    }
    write!(
        output,
        "    default:\n      result.domain = {prefix}_RUNTIME_UNKNOWN_MESSAGE;\n      break;\n  }}\n\nrelease_event:\n  wl_event_release(ctx, event);\n  result.event_consumed = 1U;\n  return result;\n}}\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        output.push('\n');
        emit_retained_implementation(&mut output, module, route);
    }
    for service in &profile.rpc_services {
        output.push('\n');
        emit_rpc_implementation(
            &mut output,
            module,
            &prefix,
            codec_module,
            &upper_snake(codec_module),
            service,
        );
        if profile.has_rpc_client() {
            let name = c_identifier(&service.name);
            let response = crate::codegen::type_name(&service.response_name);
            writeln!(output, "wl_rpc_err_t {module}_{name}_client_response(const {module}_runtime_t *runtime, uint32_t operation_id, const {response}_value_t **out_response) {{\n  wl_rpc_client_result_t client;\n  wl_rpc_err_t error;\n  if (out_response == NULL) return WL_RPC_ERR_INVALID_ARG;\n  *out_response = NULL;\n  error = {module}_{name}_client_inspect(runtime, operation_id, &client);\n  if (error != WL_RPC_OK) return error;\n  if (!client.response_valid || runtime->rpc_results == NULL || client.result_index >= runtime->rpc_result_count) return WL_RPC_ERR_INVALID_STATE;\n  *out_response = &runtime->rpc_results[client.result_index].{name};\n  return WL_RPC_OK;\n}}\n").unwrap();
        }
    }
    output.push('\n');
    emit_pump_implementation(&mut output, profile, module);
    if !profile.rpc_services.is_empty() {
        super::layout::mark_rpc(output, module, profile)
    } else {
        output
    }
}

// Preserve diagnostics and RX ownership for known messages in an omitted role.
// Do not decode them or retain references to the opposite role's codecs.
fn emit_disabled_rpc_case(
    output: &mut String,
    prefix: &str,
    message_id: u16,
    delivery: crate::profile_semantic::DeliveryPolicy,
) {
    let event = super::rpc::delivery_event(delivery);
    writeln!(output, "    case {message_id}U:\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      result.domain = event->type == {event} ? {prefix}_RUNTIME_MISSING_ROUTE : {prefix}_RUNTIME_DELIVERY_MISMATCH;\n      break;").unwrap();
}

pub(super) fn emit_result_str_implementation(output: &mut String, module: &str, prefix: &str) {
    write!(
        output,
        "const char *{module}_runtime_result_str(const {module}_runtime_result_t *result) {{\n  if (result == NULL) return \"null result\";\n  switch (result->domain) {{\n    case {prefix}_RUNTIME_OK: return \"ok\";\n    case {prefix}_RUNTIME_NON_RX: return \"non-rx event\";\n    case {prefix}_RUNTIME_UNKNOWN_MESSAGE: return \"unknown message\";\n    case {prefix}_RUNTIME_MISSING_ROUTE: return \"missing route\";\n    case {prefix}_RUNTIME_MISSING_SCRATCH: return \"missing scratch\";\n    case {prefix}_RUNTIME_DELIVERY_MISMATCH: return \"delivery mismatch\";\n    case {prefix}_RUNTIME_CODEC_ERROR: return \"codec error\";\n    case {prefix}_RUNTIME_STORAGE_ERROR: return \"storage error\";\n    case {prefix}_RUNTIME_RPC_ERROR: return \"rpc error\";\n    case {prefix}_RUNTIME_CORE_ERROR: return \"core error\";\n    case {prefix}_RUNTIME_APPLICATION_ERROR: return \"application error\";\n    case {prefix}_RUNTIME_INVALID_ARGUMENT: return \"invalid argument\";\n    default: return \"unknown runtime result\";\n  }}\n}}\n\n"
    )
    .unwrap();
}
