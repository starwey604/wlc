//! Static default RPC storage and role assembly. The core async dispatcher owns
//! admission/completion; generated glue only supplies typed snapshots/callbacks.
use crate::{
    codegen::{c_identifier, type_name},
    profile_semantic::BindingProfileModel,
};
use std::{collections::HashMap, fmt::Write};

pub(crate) fn assemble(
    mut out: String,
    profile: &BindingProfileModel,
    maxima: &HashMap<u16, Option<u64>>,
    module: &str,
) -> String {
    let managed = profile
        .rpc_services
        .iter()
        .any(|service| service.is_managed());
    let mut capacity =
        "#define @P@_ENDPOINT_RUNTIME_CAPACITY @P@_RUNTIME_DEFAULT_STORAGE_CAPACITY".to_owned();
    let mut state = String::new();
    let mut handlers = String::new();
    let mut config = String::new();
    let mut bind = String::new();
    let mut defaults = String::new();
    let mut initialize = String::new();
    let close_begin = "  if (endpoint->private_state.stepping || endpoint->private_state.closing) return WL_ERR_REENTRANT;\n  endpoint->private_state.closing = true;".to_owned();
    let mut close_end = String::new();
    if managed {
        let request_capacity = profile
            .rpc_services
            .iter()
            .filter(|service| service.is_managed())
            .map(|service| maxima[&service.request_id].unwrap() + service.metadata_size())
            .max()
            .unwrap();
        let response_capacity = profile
            .rpc_services
            .iter()
            .map(|service| maxima[&service.response_id].unwrap() + service.metadata_size())
            .max()
            .unwrap();
        capacity = format!(
            "/* Set consistently for every TU using this endpoint; no runtime allocation. */\n#ifndef @P@_ENDPOINT_RPC_CAPACITY\n#define @P@_ENDPOINT_RPC_CAPACITY 4U\n#endif\n#if @P@_ENDPOINT_RPC_CAPACITY < 1 || @P@_ENDPOINT_RPC_CAPACITY > 65535\n#error \"endpoint RPC capacity must be 1..65535\"\n#endif\n#define @P@_ENDPOINT_REQUEST_CAPACITY {request_capacity}U\n#define @P@_ENDPOINT_RUNTIME_CAPACITY (@P@_RUNTIME_DEFAULT_STORAGE_CAPACITY + (@P@_ENDPOINT_RPC_CAPACITY - 1U) * (sizeof(wl_rpc_client_slot_t) + sizeof(wl_rpc_server_pending_slot_t) + sizeof(wl_rpc_server_cache_slot_t) + 2U * {response_capacity}U))"
        );
        state.push_str("    uint64_t incarnation;\n    bool sync_waiting;\n    wl_rpc_async_t async;\n    wl_rpc_async_slot_t submissions[@P@_ENDPOINT_RPC_CAPACITY];\n    uint8_t requests[@P@_ENDPOINT_RPC_CAPACITY][@P@_ENDPOINT_REQUEST_CAPACITY];\n    wl_rpc_completion_t completion;\n    union {\n");
        config.push_str("  if (endpoint->private_state.closing) return WL_ERR_REENTRANT;\n  if (runtime_config.rpc_client_slot_count > @P@_ENDPOINT_RPC_CAPACITY ||\n      runtime_config.rpc_server_pending_slot_count > @P@_ENDPOINT_RPC_CAPACITY ||\n      runtime_config.rpc_server_cache_slot_count > @P@_ENDPOINT_RPC_CAPACITY) return WL_ERR_INVALID_ARG;\n");
        for service in &profile.rpc_services {
            let s = c_identifier(&service.name);
            if !service.is_managed() {
                continue;
            }
            let req = type_name(&service.request_name);
            let res = type_name(&service.response_name);
            writeln!(
                state,
                "      struct {{ {req}_value_t request; {res}_value_t response; }} {s};"
            )
            .unwrap();
            writeln!(
                handlers,
                "  {module}_{s}_handler_fn on_{s};\n  void *{s}_user_data;"
            )
            .unwrap();
            writeln!(config, "  if (config->on_{s} != NULL && runtime_config.{s}_request_handler != NULL) return WL_ERR_INVALID_ARG;\n  if (config->on_{s} != NULL || runtime_config.{s}_request_handler != NULL) runtime_config.rpc_server_enabled = 1U;").unwrap();
            writeln!(bind, "  endpoint->private_state.instance.runtime.{s}.value_handler = config->on_{s};\n  endpoint->private_state.instance.runtime.{s}.value_user_data = config->{s}_user_data;\n  endpoint->private_state.instance.runtime.{s}.request_value = &endpoint->private_state.values.{s}.request;\n  endpoint->private_state.instance.runtime.{s}.response_value = &endpoint->private_state.values.{s}.response;").unwrap();
        }
        state.push_str("    } values;");
        defaults.push_str("  config->advanced.rpc_client_enabled = 1U;\n  config->advanced.rpc_client_slot_count = @P@_ENDPOINT_RPC_CAPACITY;\n  config->advanced.rpc_server_pending_slot_count = @P@_ENDPOINT_RPC_CAPACITY;\n  config->advanced.rpc_server_cache_slot_count = @P@_ENDPOINT_RPC_CAPACITY;\n  config->advanced.rpc_server_pending_timeout_ms = 1000U;\n  config->advanced.rpc_server_cache_ttl_ms = 10000U;\n  config->advanced.rpc_server_cache_policy = WL_RPC_CACHE_EVICT_OLDEST;");
        initialize.push_str("  if (runtime_config.rpc_client_enabled) {\n    result = wl_rpc_async_init(&endpoint->private_state.async,\n        wl_endpoint_link(&endpoint->private_state.owner), endpoint->private_state.instance.runtime.rpc_client,\n        endpoint->private_state.submissions, runtime_config.rpc_client_slot_count,\n        endpoint->private_state.requests[0], sizeof(endpoint->private_state.requests),\n        @P@_ENDPOINT_REQUEST_CAPACITY, endpoint->private_state.incarnation);\n    if (result != WL_OK) { wl_endpoint_close(&endpoint->private_state.owner); return result; }\n    endpoint->private_state.instance.runtime.rpc_async = &endpoint->private_state.async;\n  }");
        close_end.push_str("  {\n    int error = wl_rpc_async_close(&endpoint->private_state.async);\n    endpoint->private_state.closing = false;\n    if (error != WL_OK) return error;\n  }");
    }
    for (key, value) in [
        ("RPC_CAPACITY", capacity),
        ("RPC_STATE", state),
        ("RPC_HANDLERS", handlers),
        ("RPC_DEFAULTS", defaults),
        ("RPC_CONFIG_INIT", config),
        ("RPC_BIND_VALUES", bind),
        ("RPC_ASYNC_INIT", initialize),
        ("RPC_CLOSE_BEGIN", close_begin),
        ("RPC_CLOSE_END", close_end),
    ] {
        out = out.replace(&format!("@{key}@"), &value);
    }
    if managed {
        out.push_str("\n/* Optional cancellation; completion still arrives once, with no release. */\nstatic inline wl_err_t @M@_endpoint_cancel(@M@_endpoint_t *endpoint, const wl_rpc_call_t *call) {\n  if (endpoint == NULL || call == NULL) return WL_ERR_INVALID_ARG;\n  if (endpoint->private_state.closing || wl_endpoint_link(@M@_endpoint_handle(endpoint)) == NULL) return WL_ERR_NOT_INITIALIZED;\n  return wl_rpc_async_cancel(&endpoint->private_state.async, call);\n}\n");
    }
    out
}
