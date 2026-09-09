// SPDX-License-Identifier: Apache-2.0
use crate::{
    codegen::type_name,
    profile_semantic::{DeliveryPolicy, SendRoute},
};
use std::fmt::Write;

pub(super) fn emit_type(out: &mut String, module: &str, route: &SendRoute) {
    let name = type_name(&route.message_name);
    writeln!(out, "/* Borrowed input and nested bytes/strings expire when the callback returns.\n * Return zero on success; nonzero is an application error. Do not release RX\n * or reenter dispatch. Deferred work must copy to caller-owned storage. */\ntypedef int32_t (*{module}_{name}_direct_fn)(void *user_data, const {name}_t *message, wl_delivery_t delivery);\ntypedef struct {{\n  {module}_{name}_direct_fn handler;\n  void *user_data;\n  {name}_t *scratch;\n}} {module}_{name}_direct_t;\n").unwrap();
}

pub(super) fn emit_case(out: &mut String, prefix: &str, route: &SendRoute) {
    let name = type_name(&route.message_name);
    let (event, delivery) = match route.delivery {
        DeliveryPolicy::Reliable => ("WL_EVT_RELIABLE_RX", "WL_DELIVERY_RELIABLE"),
        DeliveryPolicy::Unreliable => ("WL_EVT_UNRELIABLE_RX", "WL_DELIVERY_UNRELIABLE"),
    };
    writeln!(out, "    case {}U: {{\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_DIRECT;\n      if (event->type != {event}) {{ result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH; break; }}\n      if (runtime->{name}_direct.handler == NULL) {{ result.domain = {prefix}_RUNTIME_MISSING_ROUTE; break; }}\n      if (runtime->{name}_direct.scratch == NULL) {{ result.domain = {prefix}_RUNTIME_MISSING_SCRATCH; break; }}\n      {name}_clear(runtime->{name}_direct.scratch);\n      result.detail.direct.codec_status = {name}_decode(event->payload, event->payload_len, runtime->{name}_direct.scratch);\n      if (result.detail.direct.codec_status != WL_CODEC_OK) {{ result.domain = {prefix}_RUNTIME_CODEC_ERROR; break; }}\n      result.detail.direct.application_result = runtime->{name}_direct.handler(runtime->{name}_direct.user_data, runtime->{name}_direct.scratch, {delivery});\n      result.domain = result.detail.direct.application_result == 0 ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_APPLICATION_ERROR;\n      break;\n    }}", route.message_id).unwrap();
}
