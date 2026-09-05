//! Runtime-owned RPC metadata. The ordinary business codec is never modified.
use crate::{
    codegen::{c_identifier, type_name, upper_snake},
    profile_semantic::{DeliveryPolicy, RpcService},
};

fn expand(template: &str, module: &str, service: &RpcService) -> String {
    template
        .replace("@M@", module)
        .replace("@P@", &upper_snake(module))
        .replace("@S@", &c_identifier(&service.name))
        .replace("@REQ@", &type_name(&service.request_name))
        .replace("@RES@", &type_name(&service.response_name))
        .replace("@REQ_ID@", &format!("{}U", service.request_id))
        .replace("@RES_ID@", &format!("{}U", service.response_id))
        .replace("@REQ_EVENT@", event(service.request_delivery))
        .replace("@RES_EVENT@", event(service.response_delivery))
        .replace("@REQ_DELIVERY@", delivery(service.request_delivery))
        .replace("@TRANSITION@", match service.request_delivery {
            DeliveryPolicy::Reliable => "wl_rpc_client_bind_tx(runtime->rpc_client, operation_id, result.detail.rpc.handle)",
            DeliveryPolicy::Unreliable => "wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)",
        })
}

fn event(policy: DeliveryPolicy) -> &'static str {
    match policy {
        DeliveryPolicy::Reliable => "WL_EVT_RELIABLE_RX",
        DeliveryPolicy::Unreliable => "WL_EVT_UNRELIABLE_RX",
    }
}
fn delivery(policy: DeliveryPolicy) -> &'static str {
    match policy {
        DeliveryPolicy::Reliable => "WL_DELIVERY_RELIABLE",
        DeliveryPolicy::Unreliable => "WL_DELIVERY_UNRELIABLE",
    }
}

pub(crate) fn header_types(module: &str, codec: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_types.h.in"), module, service).replace("@CODEC@", codec)
}
pub(crate) fn header_functions(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_functions.h.in"), module, service)
}
pub(crate) fn helpers(module: &str) -> String {
    include_str!("managed_rpc_helpers.c.in").replace("@M@", module)
}
pub(crate) fn request_case(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_request.c.in"), module, service)
}
pub(crate) fn response_case(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_response.c.in"), module, service)
}
pub(crate) fn implementation(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc.c.in"), module, service)
}
pub(crate) fn endpoint(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_endpoint.h.in"), module, service)
}
