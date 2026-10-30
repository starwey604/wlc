//! Runtime-owned RPC metadata. The ordinary business codec is never modified.

use crate::{
    codegen::{c_identifier, type_name, upper_snake},
    profile_semantic::{BindingProfileModel, DeliveryPolicy, RpcGuarantee, RpcService},
    template::render,
};

fn expand(template: &str, module: &str, service: &RpcService) -> String {
    expand_with(template, module, service, &[])
}

fn expand_with(
    template: &str,
    module: &str,
    service: &RpcService,
    extra: &[(&str, &str)],
) -> String {
    let prefix = upper_snake(module);
    let name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    let request_prefix = upper_snake(&service.request_name);
    let response_prefix = upper_snake(&service.response_name);
    let request_id = format!("{}U", service.request_id);
    let response_id = format!("{}U", service.response_id);
    let mut values = vec![
        ("M", module),
        ("P", &prefix),
        ("S", &name),
        ("REQ", &request),
        ("RES", &response),
        ("REQ_P", &request_prefix),
        ("RES_P", &response_prefix),
        ("REQ_ID", &request_id),
        ("RES_ID", &response_id),
        ("REQ_EVENT", event(service.request_delivery)),
        ("RES_EVENT", event(service.response_delivery)),
        ("REQ_DELIVERY", delivery(service.request_delivery)),
        (
            "TRANSITION",
            match service.request_delivery {
                DeliveryPolicy::Reliable => {
                    "wl_rpc_client_bind_tx(runtime->rpc_client, operation_id, result.detail.rpc.handle)"
                }
                DeliveryPolicy::Unreliable => {
                    "wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)"
                }
            },
        ),
    ];
    values.extend_from_slice(extra);
    render(template, &values)
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
    expand_with(
        include_str!("managed_rpc_types.h.in"),
        module,
        service,
        &[("CODEC", codec)],
    )
}
pub(crate) fn header_functions(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_functions.h.in"), module, service)
}
pub(crate) fn helpers(module: &str, profile: &BindingProfileModel) -> String {
    let prefix = upper_snake(module);
    let mut admission = String::new();
    if profile.has_replay() {
        admission.push_str(include_str!("managed_rpc_begin_replay.c.in"));
    }
    if profile.has_correlate() {
        admission.push_str(include_str!("managed_rpc_begin_correlate.c.in"));
    }
    let admission = render(&admission, &[("M", module), ("P", &prefix)]);
    render(include_str!("managed_rpc_helpers.c.in"), &[("M", module)])
        + &render(
            include_str!("managed_rpc_server.c.in"),
            &[("M", module), ("P", &prefix), ("ADMISSION", &admission)],
        )
}
pub(crate) fn request_case(module: &str, service: &RpcService) -> String {
    let replay = service.guarantee == RpcGuarantee::Replay;
    let fingerprint = if replay {
        expand(
            include_str!("managed_rpc_fingerprint.c.in"),
            module,
            service,
        )
    } else {
        String::new()
    };
    let seed = format!(".request_fingerprint = {module}_rpc_fingerprint_seed");
    expand_with(
        include_str!("managed_rpc_request.c.in"),
        module,
        service,
        &[
            ("FINGERPRINT_INIT", if replay { &seed } else { "0" }),
            ("FINGERPRINT", &fingerprint),
            ("BEGIN_SUFFIX", if replay { "" } else { "_correlate" }),
        ],
    )
}
pub(crate) fn response_case(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_response.c.in"), module, service)
}
pub(crate) fn implementation(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc.c.in"), module, service) + "\n"
}
pub(crate) fn endpoint(module: &str, service: &RpcService) -> String {
    expand(include_str!("managed_rpc_endpoint.h.in"), module, service)
}
pub(crate) fn async_endpoint(module: &str, service: &RpcService) -> String {
    expand(include_str!("rpc_async_endpoint.h.in"), module, service)
        + &expand(include_str!("rpc_sync_endpoint.h.in"), module, service)
}

pub(crate) fn private_source(
    schema: &crate::semantic::SemanticModel,
    module: &str,
    profile: &BindingProfileModel,
) -> String {
    crate::codegen::private_source(schema, module, profile)
}
