// SPDX-License-Identifier: Apache-2.0
//! Static RPC resources. Pending transmission and completed replay history are
//! distinct capabilities; neither is inferred from a runtime cache switch.
use std::collections::BTreeSet;

use thiserror::Error;

use crate::{
    BindingProfileModel, SemanticModel,
    codec_plan::{CodecPlan, CodecProfile},
    codegen::c_identifier,
    profile_semantic::RpcGuarantee,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcMethodPlan {
    pub name: String,
    pub request_id: u16,
    pub response_id: u16,
    pub response_name: String,
    pub codec: CodecProfile,
    pub guarantee: RpcGuarantee,
    /// Business payload bounds before RPC metadata or rejection storage.
    pub request_body_capacity: Option<u64>,
    pub response_body_capacity: Option<u64>,
    /// Wire bounds, including metadata and the minimum rejection response.
    pub request_capacity: Option<u64>,
    pub response_capacity: Option<u64>,
    pub response_has_owned_value: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcEndpointPlan {
    pub methods: Vec<RpcMethodPlan>,
    /// Members of one correctly aligned generated union per client slot.
    pub client_result_methods: Vec<String>,
    /// Every admitted server execution needs stable response/TX storage.
    pub pending_response_methods: Vec<String>,
    /// Only these methods pay for fingerprint and completed response history.
    pub replay_methods: Vec<String>,
    pub request_snapshot_capacity: Option<u64>,
    pub pending_response_capacity: Option<u64>,
    pub replay_response_capacity: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("RPC plan failed: {0}")]
pub struct RpcPlanError(pub String);

impl RpcEndpointPlan {
    pub fn new(
        schema: &SemanticModel,
        profile: &BindingProfileModel,
    ) -> Result<Self, RpcPlanError> {
        let mut plan = Self {
            methods: Vec::new(),
            client_result_methods: Vec::new(),
            pending_response_methods: Vec::new(),
            replay_methods: Vec::new(),
            request_snapshot_capacity: None,
            pending_response_capacity: None,
            replay_response_capacity: None,
        };
        let mut services = profile.rpc_services.iter().collect::<Vec<_>>();
        services.sort_by(|a, b| a.name.cmp(&b.name));
        let mut members = BTreeSet::new();
        let mut request_bounds = Vec::new();
        let mut pending_bounds = Vec::new();
        let mut replay_bounds = Vec::new();
        for service in services {
            if !members.insert(c_identifier(&service.name)) {
                return Err(RpcPlanError(format!(
                    "method `{}` has a colliding result member name",
                    service.name
                )));
            }
            let codec = CodecPlan::selected(
                schema,
                service.codec,
                [service.request_id, service.response_id],
            )
            .map_err(|e| RpcPlanError(e.to_string()))?;
            let request = &codec.messages[&service.request_id];
            let response = &codec.messages[&service.response_id];
            if request.message.name != service.request_name
                || response.message.name != service.response_name
            {
                return Err(RpcPlanError(format!(
                    "method `{}` message names/IDs do not match the schema",
                    service.name
                )));
            }
            let bound = |body: Option<u64>, minimum| -> Result<Option<u64>, RpcPlanError> {
                body.map(|body| {
                    body.checked_add(service.metadata_size())
                        .map(|n| n.max(minimum))
                        .ok_or_else(|| {
                            RpcPlanError(format!("metadata size overflow for `{}`", service.name))
                        })
                })
                .transpose()
            };
            let request_capacity = bound(request.max_encoded_size, 0)?;
            let response_capacity =
                bound(response.max_encoded_size, service.minimum_response_size())?;
            if profile.has_rpc_client() {
                plan.client_result_methods.push(service.name.clone());
                request_bounds.push(request_capacity);
            }
            if profile.has_rpc_server() {
                plan.pending_response_methods.push(service.name.clone());
                pending_bounds.push(response_capacity);
                if service.guarantee == RpcGuarantee::Replay {
                    plan.replay_methods.push(service.name.clone());
                    replay_bounds.push(response_capacity);
                }
            }
            plan.methods.push(RpcMethodPlan {
                name: service.name.clone(),
                request_id: service.request_id,
                response_id: service.response_id,
                response_name: service.response_name.clone(),
                codec: service.codec,
                guarantee: service.guarantee,
                request_body_capacity: request.max_encoded_size,
                response_body_capacity: response.max_encoded_size,
                request_capacity,
                response_capacity,
                response_has_owned_value: response.has_owned_value(),
            });
        }
        // An empty group has no resources. An unbounded member cannot be hidden
        // by a smaller bounded method; generation must request explicit bounds.
        let maximum = |bounds: Vec<Option<u64>>| {
            bounds
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .and_then(|bounds| bounds.into_iter().max())
        };
        plan.request_snapshot_capacity = maximum(request_bounds);
        plan.pending_response_capacity = maximum(pending_bounds);
        plan.replay_response_capacity = maximum(replay_bounds);
        Ok(plan)
    }

    /// Backends retain profile declaration order and query these shared facts.
    pub(crate) fn method(&self, name: &str) -> &RpcMethodPlan {
        &self.methods[self
            .methods
            .binary_search_by(|method| method.name.as_str().cmp(name))
            .expect("validated profile method in RPC plan")]
    }

    /// All declared methods participate, even for a client-only profile: the
    /// advanced runtime still emits server configuration/default declarations.
    pub(crate) fn response_capacity_for(&self, guarantee: RpcGuarantee) -> Option<u64> {
        self.methods
            .iter()
            .filter(|method| method.guarantee == guarantee)
            .map(|method| method.response_capacity)
            .collect::<Option<Vec<_>>>()
            .and_then(|bounds| bounds.into_iter().max())
    }

    /// Validate execution support without rendering a target declaration.
    /// Planning alone can still describe unavailable, unbounded owned results.
    pub fn validate_client_results(&self) -> Result<(), RpcPlanError> {
        for method in self
            .methods
            .iter()
            .filter(|m| self.client_result_methods.contains(&m.name))
        {
            if !method.response_has_owned_value {
                return Err(RpcPlanError(format!(
                    "typed result for `{}` requires a statically bounded response",
                    method.name
                )));
            }
        }
        Ok(())
    }
}
