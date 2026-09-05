//! Default, allocation-free endpoint facade over the advanced runtime API.
use std::fmt::Write;

use crate::{
    codegen::{c_identifier, static_max_encoded_sizes, type_name, upper_snake},
    profile_semantic::{BindingProfileModel, DeliveryPolicy, RetainedRouteKind},
    semantic::{SemanticModel, Symbol},
};

pub(crate) fn emit(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    codec: &str,
    module: &str,
    has_runtime_storage: bool,
) -> String {
    let prefix = upper_snake(module);
    let messages = schema
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some(message),
            Symbol::Enum(_) => None,
        })
        .collect::<Vec<_>>();
    let maxima = static_max_encoded_sizes(&messages);
    let mut selected = profile
        .retained_routes
        .iter()
        .map(|route| (route.message_id, 0))
        .chain(profile.rpc_services.iter().flat_map(|service| {
            [
                (service.request_id, service.metadata_size()),
                (service.response_id, service.metadata_size()),
            ]
        }));
    let maximum = selected.try_fold(1_u64, |bound, (id, overhead)| {
        maxima
            .get(&id)
            .copied()
            .flatten()
            .map(|value| bound.max(value + overhead))
    });
    let Some(maximum) = maximum.filter(|value| *value <= 2048 && has_runtime_storage) else {
        return format!(
            "/* No finite one-frame schema bound: use custom link/runtime storage. */\n#define {prefix}_HAS_DEFAULT_ENDPOINT 0\n\n"
        );
    };
    let rpc_check = if profile.rpc_services.is_empty() {
        String::new()
    } else {
        format!(
            "  if (endpoint->private_state.pump.last_service_result != WL_RPC_OK) {{\n    if ({module}_runtime_result_ok(&endpoint->private_state.result)) {{\n      endpoint->private_state.result.domain = {prefix}_RUNTIME_RPC_ERROR;\n      endpoint->private_state.result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      endpoint->private_state.result.detail.rpc.rpc_result = endpoint->private_state.pump.last_service_result;\n    }}\n    return WL_ERR_INVALID_STATE;\n  }}"
        )
    };
    let managed = profile
        .rpc_services
        .iter()
        .any(|service| service.is_managed());
    let mut output = include_str!("endpoint.h.in")
        .replace("@RPC_CHECK@", &rpc_check)
        .replace("@RPC_STATE@", if managed { "    uint64_t incarnation;" } else { "" })
        .replace("@RPC_BEGIN_INIT@", if managed {
            "  if (endpoint->private_state.incarnation == UINT64_MAX) return WL_ERR_INVALID_STATE;\n  ++endpoint->private_state.incarnation;"
        } else { "" })
        .replace("@RPC_INCARNATION@", if !managed { "" } else {
            "  endpoint->private_state.instance.runtime.rpc_incarnation = endpoint->private_state.incarnation;"
        })
        .replace("@M@", module)
        .replace("@P@", &prefix)
        .replace("@MAX@", &maximum.to_string());
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        let delivery = match route.delivery {
            DeliveryPolicy::Unreliable => "WL_DELIVERY_UNRELIABLE",
            DeliveryPolicy::Reliable => "WL_DELIVERY_RELIABLE",
        };
        writeln!(output, "/* Delivery follows this binding. Use codec sends to override explicitly. */\nstatic inline {codec}_send_result_t {module}_endpoint_send_{message}({module}_endpoint_t *endpoint, const {message}_t *message) {{\n  return {codec}_{message}_send(wl_endpoint_link({module}_endpoint_handle(endpoint)), message, {delivery});\n}}\n\n/* Copy an owned value and release its lease internally. NO_DATA leaves out unchanged. */\nstatic inline wl_err_t {module}_endpoint_read_{message}({module}_endpoint_t *endpoint, {message}_t *out) {{\n  {module}_{message}_{kind}_view_t view;\n  {module}_runtime_t *runtime = {module}_endpoint_runtime(endpoint);\n  int result;\n  if (out == NULL) return WL_ERR_INVALID_ARG;\n  if (runtime == NULL) return WL_ERR_NOT_INITIALIZED;\n  result = {module}_{message}_{kind}_acquire(runtime, &view);\n  if (result != WL_OK) return result;\n  *out = *view.value;\n  return {module}_{message}_{kind}_release(runtime, &view);\n}}\n").unwrap();
    }
    for service in &profile.rpc_services {
        if service.is_managed() {
            output.push_str(&crate::managed_rpc_codegen::endpoint(module, service));
            continue;
        }
        let name = c_identifier(&service.name);
        let request = type_name(&service.request_name);
        let response = type_name(&service.response_name);
        writeln!(output, "static inline {module}_runtime_result_t {module}_endpoint_{name}_start({module}_endpoint_t *endpoint, const {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms) {{\n  return {module}_{name}_client_start(wl_endpoint_link({module}_endpoint_handle(endpoint)), {module}_endpoint_runtime(endpoint), request, timeout_ms, now_ms);\n}}\n\nstatic inline wl_rpc_err_t {module}_endpoint_{name}_inspect({module}_endpoint_t *endpoint, uint32_t operation_id, wl_rpc_client_result_t *result) {{\n  return {module}_{name}_client_inspect({module}_endpoint_runtime(endpoint), operation_id, result);\n}}\n\nstatic inline wl_rpc_err_t {module}_endpoint_{name}_release({module}_endpoint_t *endpoint, uint32_t operation_id) {{\n  return {module}_{name}_client_release({module}_endpoint_runtime(endpoint), operation_id);\n}}\n\nstatic inline {module}_runtime_result_t {module}_endpoint_{name}_complete({module}_endpoint_t *endpoint, const wl_rpc_server_request_t *request, const {response}_t *response, wl_time_ms_t now_ms) {{\n  return {module}_{name}_server_complete({module}_endpoint_runtime(endpoint), request, response, now_ms);\n}}\n").unwrap();
    }
    output
}
