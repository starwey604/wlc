// SPDX-License-Identifier: Apache-2.0
//! Version-2 profile contract for the isolated static-kernel prototype.
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    BindingProfile, BindingProfileModel, SemanticModel,
    ast::Spanned,
    codec_plan::CodecProfile,
    endpoint_layout::{EndpointEnvelope, EndpointRpcRole},
    profile::BindingDeclaration,
    profile_semantic::DeliveryPolicy,
};

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("static kernel: {0}")]
#[diagnostic(code(wlc::kernel))]
pub struct KernelError(pub String);

/// Only constructed after the new semantic contract has been checked.
/// The shared binding model supplies name resolution and codec selection;
/// its legacy execution modes are never generated or linked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelProfile {
    pub(crate) bindings: BindingProfileModel,
}

pub fn analyze_kernel_profile(
    profile: &BindingProfile,
    schema: &SemanticModel,
) -> Result<KernelProfile, KernelError> {
    if profile.version.value != 2 {
        return Err(KernelError(
            "requires profile version 2; migrate explicitly".into(),
        ));
    }
    let mut normalized = profile.clone();
    for declaration in &mut normalized.bindings {
        match declaration {
            BindingDeclaration::Latest(_) | BindingDeclaration::Fifo(_) => {
                return Err(KernelError(
                    "LATEST/FIFO are outside the base kernel".into(),
                ));
            }
            BindingDeclaration::Send(route) | BindingDeclaration::Direct(route) => {
                if route.delivery.value != "unreliable" {
                    return Err(KernelError(
                        "reliable delivery is removed; no silent downgrade".into(),
                    ));
                }
            }
            BindingDeclaration::Rpc(rpc) => {
                if rpc.guarantee.is_some() {
                    return Err(KernelError(
                        "RPC guarantee/replay declarations are removed".into(),
                    ));
                }
                if rpc.request_operation_id.is_some()
                    || rpc.response_operation_id.is_some()
                    || rpc.response_status.is_some()
                {
                    return Err(KernelError("mapped RPC declarations are removed".into()));
                }
                if rpc.request_delivery.value != "unreliable"
                    || rpc.response_delivery.value != "unreliable"
                {
                    return Err(KernelError(
                        "reliable RPC delivery is removed; no silent downgrade".into(),
                    ));
                }
                // Reuse the existing name/type/codec resolver, after rejecting
                // every removed declaration. This does not choose an execution
                // mode: only the new kernel generator accepts KernelProfile.
                rpc.guarantee = Some(Spanned {
                    value: "correlate".into(),
                    span: rpc.name.span,
                });
            }
        }
    }
    normalized.version.value = 1;
    let mut bindings = crate::analyze_binding_profile(&normalized, schema).map_err(|errors| {
        KernelError(
            errors
                .errors()
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    bindings.version = 2;
    validate(&bindings)?;
    Ok(KernelProfile { bindings })
}

pub fn compose_kernel_profiles(profiles: &[KernelProfile]) -> Result<KernelProfile, KernelError> {
    let shared = profiles
        .iter()
        .map(|p| {
            let mut p = p.bindings.clone();
            p.version = 1;
            p
        })
        .collect::<Vec<_>>();
    let mut bindings =
        crate::compose_binding_profiles(&shared).map_err(|e| KernelError(e.to_string()))?;
    bindings.version = 2;
    validate(&bindings)?;
    Ok(KernelProfile { bindings })
}

fn validate(bindings: &BindingProfileModel) -> Result<(), KernelError> {
    if bindings.endpoint.is_some_and(|e| {
        e.envelope != EndpointEnvelope::NativePacket || e.rpc_role != EndpointRpcRole::Both
    }) {
        return Err(KernelError(
            "K1 supports native_packet, integrity NONE and both RPC roles".into(),
        ));
    }
    for route in bindings.send_routes.iter().chain(&bindings.direct_routes) {
        if route.delivery != DeliveryPolicy::Unreliable || route.codec != CodecProfile::SchemaV1 {
            return Err(KernelError(
                "K1 ordinary messages require unreliable Schema-v1 codecs".into(),
            ));
        }
    }
    for service in &bindings.rpc_services {
        if service.codec != CodecProfile::ManagedTagged {
            return Err(KernelError(
                "K1 RPC preserves the managed tagged wire codec".into(),
            ));
        }
    }
    Ok(())
}
