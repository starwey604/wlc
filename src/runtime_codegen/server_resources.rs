// SPDX-License-Identifier: Apache-2.0
//! Server resource groups are selected statically. Pending executions share one
//! admission pool; only replay methods reserve retained history. Correlate has
//! independent stable response/TX capacity, released after transport completion.
use crate::rpc_plan::RpcEndpointPlan;
use crate::{BindingProfileModel, profile_semantic::RpcGuarantee};
use std::fmt::Write;

pub(super) struct ServerResources {
    pub replay: bool,
    pub correlate: bool,
    groups: Vec<Group>,
}
struct Group {
    count: &'static str,
    capacity: &'static str,
    slot: &'static str,
    slots: &'static str,
    bytes: &'static str,
    maximum: Option<u16>,
}
impl ServerResources {
    pub fn new(profile: &BindingProfileModel, rpc_plan: &RpcEndpointPlan) -> Self {
        let mut groups = Vec::new();
        for (guarantee, count, capacity, slot, slots, bytes) in [
            (
                RpcGuarantee::Replay,
                "rpc_server_cache_slot_count",
                "rpc_server_response_capacity",
                "wl_rpc_server_cache_slot_t",
                "rpc_server_cache_slots",
                "rpc_server_responses",
            ),
            (
                RpcGuarantee::Correlate,
                "rpc_server_correlate_slot_count",
                "rpc_server_correlate_response_capacity",
                "wl_rpc_server_response_slot_t",
                "rpc_server_correlate_slots",
                "rpc_server_correlate_responses",
            ),
        ] {
            if !rpc_plan
                .methods
                .iter()
                .any(|method| method.guarantee == guarantee)
            {
                continue;
            }
            let maximum = rpc_plan
                .response_capacity_for(guarantee)
                .and_then(|n| u16::try_from(n.max(1)).ok());
            groups.push(Group {
                count,
                capacity,
                slot,
                slots,
                bytes,
                maximum,
            });
        }
        Self {
            replay: profile.has_replay(),
            correlate: profile.has_correlate(),
            groups,
        }
    }
    pub fn config_fields(&self) -> String {
        let mut s = String::from(
            "  uint8_t rpc_server_enabled;\n  uint16_t rpc_server_pending_slot_count;\n  uint32_t rpc_server_pending_timeout_ms;\n",
        );
        for g in &self.groups {
            writeln!(s, "  uint16_t {};\n  uint16_t {};", g.count, g.capacity).unwrap();
        }
        if self.replay {
            s.push_str("  uint32_t rpc_server_cache_ttl_ms;\n  wl_rpc_cache_policy_t rpc_server_cache_policy;\n");
        }
        s
    }
    pub fn defaults(&self) -> String {
        let mut s = String::from("  config->rpc_server_pending_slot_count = 1U;\n");
        for g in &self.groups {
            writeln!(s, "  config->{} = 1U;", g.count).unwrap();
            if let Some(n) = g.maximum {
                writeln!(s, "  config->{} = {n}U;", g.capacity).unwrap();
            }
        }
        if self.replay {
            s.push_str("  config->rpc_server_cache_policy = WL_RPC_CACHE_REJECT_NEW;\n");
        }
        s
    }
    pub fn invalid_capacity(&self, base: &str) -> String {
        let mut terms = vec![format!("{base}->rpc_server_pending_slot_count == 0U")];
        for g in &self.groups {
            terms.push(format!(
                "{base}->{} == 0U || {base}->{} == 0U",
                g.count, g.capacity
            ));
        }
        if self.replay && self.correlate {
            terms.push(format!("{base}->rpc_server_correlate_slot_count > UINT16_MAX - {base}->rpc_server_cache_slot_count"));
        }
        terms.join(" || ")
    }
    pub fn storage_terms(&self, padding: &str) -> Vec<String> {
        let mut s = vec![format!(
            "({padding} + sizeof(wl_rpc_server_pending_slot_t))"
        )];
        for g in &self.groups {
            s.push(format!("({padding} + sizeof({}))", g.slot));
            s.push(format!("{}U", g.maximum.expect("default bounded storage")));
        }
        s
    }
    pub fn alignment_members(&self) -> String {
        let mut s = String::from("  wl_rpc_server_pending_slot_t rpc_server_pending_slot;\n");
        for g in &self.groups {
            writeln!(s, "  {} {};", g.slot, g.slots).unwrap();
        }
        s
    }
    pub fn layout_fields(&self) -> String {
        let mut s = String::from("  void *rpc_server_pending_slots;\n");
        for g in &self.groups {
            writeln!(
                s,
                "  void *{};\n  void *{};\n  size_t {}_size;",
                g.slots, g.bytes, g.bytes
            )
            .unwrap();
        }
        s
    }
    pub fn layout(&self, m: &str) -> String {
        let mut s = format!(
            "    if ({}) return WL_ERR_INVALID_ARG;\n    if (config->rpc_server_pending_timeout_ms >= UINT32_C(0x80000000)) return WL_ERR_INVALID_ARG;\n",
            self.invalid_capacity("config")
        );
        if self.replay {
            s.push_str("    if (config->rpc_server_cache_ttl_ms >= UINT32_C(0x80000000)) return WL_ERR_INVALID_ARG;\n    if (config->rpc_server_cache_policy != WL_RPC_CACHE_REJECT_NEW && config->rpc_server_cache_policy != WL_RPC_CACHE_EVICT_OLDEST) return WL_ERR_INVALID_ARG;\n");
        }
        s.push_str("    if (alignment < _Alignof(wl_rpc_server_pending_slot_t)) alignment = _Alignof(wl_rpc_server_pending_slot_t);\n");
        writeln!(s, "    result = {m}_runtime_storage_region(&cursor, _Alignof(wl_rpc_server_pending_slot_t), config->rpc_server_pending_slot_count, sizeof(wl_rpc_server_pending_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_server_pending_slots, NULL);\n    if (result != WL_OK) return result;").unwrap();
        for g in &self.groups {
            writeln!(
                s,
                "    if (alignment < _Alignof({})) alignment = _Alignof({});",
                g.slot, g.slot
            )
            .unwrap();
            writeln!(s, "    result = {m}_runtime_storage_region(&cursor, _Alignof({}), config->{}, sizeof({}), out_layout == NULL ? NULL : &out_layout->{}, NULL);\n    if (result != WL_OK) return result;", g.slot, g.count, g.slot, g.slots).unwrap();
            writeln!(s, "    result = {m}_runtime_storage_region(&cursor, 1U, config->{}, config->{}, out_layout == NULL ? NULL : &out_layout->{}, out_layout == NULL ? NULL : &out_layout->{}_size);\n    if (result != WL_OK) return result;", g.count, g.capacity, g.bytes, g.bytes).unwrap();
        }
        s
    }
    pub fn diagnostics(&self, m: &str, p: &str) -> String {
        let mut fields = vec!["rpc_server_pending_slot_count"];
        for g in &self.groups {
            fields.extend([g.count, g.capacity]);
        }
        let mut s = String::new();
        for field in fields {
            writeln!(s, "  if (config->rpc_server_enabled && config->{field} == 0U) return {m}_runtime_init_failure(diagnostic, {p}_RUNTIME_INIT_RPC_SERVER_CAPACITY, \"{field}\", 1U, 0U, WL_ERR_INVALID_ARG);").unwrap();
        }
        let mut timers = vec!["rpc_server_pending_timeout_ms"];
        if self.replay {
            timers.push("rpc_server_cache_ttl_ms");
        }
        for field in timers {
            writeln!(s, "  if (config->rpc_server_enabled && config->{field} >= UINT32_C(0x80000000)) return {m}_runtime_init_failure(diagnostic, {p}_RUNTIME_INIT_RPC_TIMEOUT, \"{field}\", UINT32_C(0x7fffffff), config->{field}, WL_ERR_INVALID_ARG);").unwrap();
        }
        if self.replay {
            writeln!(s, "  if (config->rpc_server_enabled && config->rpc_server_cache_policy != WL_RPC_CACHE_REJECT_NEW && config->rpc_server_cache_policy != WL_RPC_CACHE_EVICT_OLDEST) return {m}_runtime_init_failure(diagnostic, {p}_RUNTIME_INIT_RPC_CACHE_POLICY, \"rpc_server_cache_policy\", 0U, (size_t)config->rpc_server_cache_policy, WL_ERR_INVALID_ARG);").unwrap();
        }
        s
    }
    pub fn init(&self, _m: &str, p: &str) -> String {
        let mut s = format!(
            "#if {p}_RUNTIME_HAS_RPC_SERVER\n  if (config->rpc_server_enabled) {{\n    const wl_rpc_server_config_t server_config = {{\n      .pending_slots = (wl_rpc_server_pending_slot_t *)layout.rpc_server_pending_slots,\n      .pending_slot_count = config->rpc_server_pending_slot_count,\n      .pending_timeout_ms = config->rpc_server_pending_timeout_ms,\n"
        );
        if self.replay {
            s.push_str("      .cache_slots = (wl_rpc_server_cache_slot_t *)layout.rpc_server_cache_slots,\n      .cache_slot_count = config->rpc_server_cache_slot_count,\n      .response_storage = (uint8_t *)layout.rpc_server_responses,\n      .response_storage_size = layout.rpc_server_responses_size,\n      .response_capacity_per_slot = config->rpc_server_response_capacity,\n      .cache_ttl_ms = config->rpc_server_cache_ttl_ms,\n      .cache_policy = config->rpc_server_cache_policy,\n");
        }
        s.push_str("    };\n");
        if self.correlate {
            s.push_str("    const wl_rpc_server_response_pool_t correlate = {\n      (wl_rpc_server_response_slot_t *)layout.rpc_server_correlate_slots,\n      config->rpc_server_correlate_slot_count,\n      (uint8_t *)layout.rpc_server_correlate_responses,\n      layout.rpc_server_correlate_responses_size,\n      config->rpc_server_correlate_response_capacity\n    };\n");
            writeln!(s, "    if (wl_rpc_server_init_with_correlate(&instance->rpc_server, &server_config, &correlate) != WL_RPC_OK) {{").unwrap();
        } else {
            writeln!(
                s,
                "    if (wl_rpc_server_init(&instance->rpc_server, &server_config) != WL_RPC_OK) {{"
            )
            .unwrap();
        }
        s.push_str("      result = WL_ERR_INVALID_ARG;\n      goto init_failed;\n    }\n    instance->runtime.rpc_server = &instance->rpc_server;\n  }\n#endif\n");
        s
    }
}
