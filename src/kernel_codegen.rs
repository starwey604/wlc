// SPDX-License-Identifier: Apache-2.0
//! Static routing over the compact C kernel; no old runtime code generation.
use crate::{
    KernelProfile, SemanticModel,
    codec_plan::{CodecPlan, CodecProfile},
    codegen::{CModel, type_name, upper_snake},
    kernel_profile::KernelError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedKernelC {
    pub files: BTreeMap<String, String>,
}

pub fn generate_kernel_c(
    schema: &SemanticModel,
    profile: &KernelProfile,
    module: &str,
) -> Result<GeneratedKernelC, KernelError> {
    let facts = CModel::new(schema, module).map_err(|e| KernelError(e.to_string()))?;
    let m = &facts.module;
    let upper = upper_snake(m);
    let p = &profile.bindings;
    let mut service_names = BTreeSet::new();
    for service in &p.rpc_services {
        if !service_names.insert(type_name(&service.name)) {
            return Err(KernelError(
                "RPC names collide after C identifier normalization".into(),
            ));
        }
    }
    if p.endpoint.is_none() {
        return Err(KernelError(
            "a native_packet endpoint declaration is required".into(),
        ));
    }
    let mut capacities = BTreeMap::new();
    let mut req_max = 17u64;
    let mut resp_max = 21u64;
    let mut msg_max = 4u64;
    for service in &p.rpc_services {
        for (id, response) in [(service.request_id, false), (service.response_id, true)] {
            let plan = CodecPlan::selected(schema, service.codec, [id])
                .map_err(|e| KernelError(e.to_string()))?;
            let body = bounded(plan.messages[&id].max_encoded_size)?;
            let frame = if response {
                (body + 17).max(21)
            } else {
                body + 17
            };
            capacities.insert(id, (body, frame, service.codec.name()));
            if response {
                resp_max = resp_max.max(frame);
            } else {
                req_max = req_max.max(frame);
            }
        }
    }
    for route in p.send_routes.iter().chain(&p.direct_routes) {
        let plan = CodecPlan::selected(schema, CodecProfile::SchemaV1, [route.message_id])
            .map_err(|e| KernelError(e.to_string()))?;
        let body = bounded(plan.messages[&route.message_id].max_encoded_size)?;
        capacities.insert(route.message_id, (body, body + 4, route.codec.name()));
        if p.send_routes
            .iter()
            .any(|r| r.message_id == route.message_id)
        {
            msg_max = msg_max.max(body + 4);
        }
    }
    if capacities.values().any(|(_, frame, _)| *frame > 2052) {
        return Err(KernelError(
            "selected message exceeds the 2048-byte native payload limit".into(),
        ));
    }
    let public =
        crate::codegen::generate_c_with_fingerprints(schema, m, Some(&BTreeSet::new()), None)
            .map_err(|e| KernelError(e.to_string()))?;
    let mut header = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#ifndef {upper}_KERNEL_H\n#define {upper}_KERNEL_H\n#include \"{m}.h\"\n#include \"wirelink/kernel.h\"\n#if WL_KERNEL_CONTRACT != 1\n#error \"static kernel contract mismatch\"\n#endif\n#define {upper}_KERNEL_PROFILE_VERSION 2U\n#define {upper}_KERNEL_REQUEST_FRAME_MAX {req_max}U\n#define {upper}_KERNEL_REPLY_FRAME_MAX {resp_max}U\n#define {upper}_KERNEL_MESSAGE_FRAME_MAX {msg_max}U\n"
    );
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let resp = type_name(&service.response_name);
        writeln!(header, "typedef void (*{m}_kernel_{name}_callback_t)(void *, wl_kernel_result_t, const {resp}_t *);").unwrap();
    }
    writeln!(header, "typedef struct {{").unwrap();
    if p.rpc_services.is_empty() && p.direct_routes.is_empty() {
        header.push_str("  uint8_t _empty;\n");
    }
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let req = type_name(&service.request_name);
        let resp = type_name(&service.response_name);
        writeln!(
            header,
            "  int32_t (*on_rpc_{name})(void *, const {req}_t *, {resp}_value_t *);"
        )
        .unwrap();
    }
    for route in &p.direct_routes {
        let name = type_name(&route.message_name);
        writeln!(
            header,
            "  void (*on_message_{name})(void *, const {name}_t *);"
        )
        .unwrap();
    }
    writeln!(header, "}} {m}_kernel_handlers_t;\ntypedef struct {{\n  wl_kernel_t core;\n  {m}_kernel_handlers_t handlers;\n  void *user;\n  union {{").unwrap();
    if p.rpc_services.is_empty() {
        header.push_str("    uint8_t _empty;\n");
    }
    for service in &p.rpc_services {
        writeln!(
            header,
            "    {}_value_t {};",
            type_name(&service.response_name),
            type_name(&service.name)
        )
        .unwrap();
    }
    writeln!(header, "  }} reply;\n  uint8_t requests[WL_KERNEL_CALLS][{req_max}U];\n  uint8_t replies[WL_KERNEL_REPLIES][{resp_max}U];\n  uint8_t message[{msg_max}U];\n}} {m}_kernel_t;\nwl_err_t {m}_kernel_init({m}_kernel_t *, wl_packet_port_t *, const {m}_kernel_handlers_t *, wl_kernel_error_fn, void *);\nwl_err_t {m}_kernel_step({m}_kernel_t *, uint32_t now);\n").unwrap();
    for service in &p.rpc_services {
        writeln!(header, "wl_err_t {m}_kernel_{}_start({m}_kernel_t *, const {}_value_t *, uint32_t now, uint32_t timeout, {m}_kernel_{}_callback_t, void *, wl_kernel_call_handle_t *);", type_name(&service.name), type_name(&service.request_name), type_name(&service.name)).unwrap();
    }
    for route in &p.send_routes {
        writeln!(
            header,
            "wl_err_t {m}_kernel_send_{}({m}_kernel_t *, const {}_value_t *);",
            type_name(&route.message_name),
            type_name(&route.message_name)
        )
        .unwrap();
        writeln!(header, "/* Borrowed input is encoded before return; no view or source bytes are retained. */\nwl_err_t {m}_kernel_send_{}_view({m}_kernel_t *, const {}_t *);", type_name(&route.message_name), type_name(&route.message_name)).unwrap();
    }
    header.push_str("#endif\n");
    let mut private = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#ifndef {upper}_KERNEL_CODECS_H\n#define {upper}_KERNEL_CODECS_H\n#include \"{m}.h\"\n"
    );
    if !p.rpc_services.is_empty() {
        private.push_str(&crate::codegen::private_source(schema, m, p));
    }
    private.push_str("#endif\n");
    let mut source = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#include \"{m}_kernel.h\"\n#include \"{m}_kernel_codecs.h\"\n#include <string.h>\n"
    );
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let req = type_name(&service.request_name);
        let resp = type_name(&service.response_name);
        for ty in [&req, &resp] {
            writeln!(source, "static wl_err_t {m}_kernel_encode_{ty}(const void *value, uint8_t *data, size_t capacity, size_t *length) {{\n  return {m}_rpc_{ty}_value_encode(value, data, capacity, length) == WL_CODEC_OK ? WL_OK : WL_ERR_CORRUPT_PAYLOAD;\n}}").unwrap();
        }
        writeln!(source, "static void {m}_kernel_complete_{name}(wl_kernel_t *core, const wl_kernel_call_t *call, wl_kernel_result_t result, const uint8_t *body, size_t length) {{\n  (void)core;\n  {resp}_t response;\n  const {resp}_t *view = NULL;\n  if (result.outcome == WL_KERNEL_SUCCESS) {{\n    if ({m}_rpc_{resp}_decode(body, length, &response) == WL_CODEC_OK) view = &response;\n    else result = (wl_kernel_result_t){{WL_KERNEL_LOCAL_ERROR, WL_ERR_CORRUPT_PAYLOAD}};\n  }}\n  (({m}_kernel_{name}_callback_t)call->callback)(call->user, result, view);\n}}\nwl_err_t {m}_kernel_{name}_start({m}_kernel_t *endpoint, const {req}_value_t *request, uint32_t now, uint32_t timeout, {m}_kernel_{name}_callback_t callback, void *user, wl_kernel_call_handle_t *out) {{\n  if (endpoint == NULL || request == NULL) return WL_ERR_INVALID_ARG;\n  return wl_kernel_call_start(&endpoint->core, {}U, {}U, request, {m}_kernel_encode_{req}, now, timeout, {m}_kernel_complete_{name}, (void (*)(void))callback, user, out);\n}}", service.request_id, service.response_id).unwrap();
    }
    for route in &p.send_routes {
        let name = type_name(&route.message_name);
        writeln!(source, "static wl_err_t {m}_kernel_encode_{name}(const void *value, uint8_t *data, size_t capacity, size_t *length) {{\n  return {name}_value_encode(value, data, capacity, length) == WL_CODEC_OK ? WL_OK : WL_ERR_CORRUPT_PAYLOAD;\n}}\nwl_err_t {m}_kernel_send_{name}({m}_kernel_t *endpoint, const {name}_value_t *value) {{\n  if (endpoint == NULL || value == NULL) return WL_ERR_INVALID_ARG;\n  return wl_kernel_send(&endpoint->core, {}U, value, {m}_kernel_encode_{name});\n}}", route.message_id).unwrap();
        writeln!(source, "static wl_err_t {m}_kernel_encode_{name}_view(const void *value, uint8_t *data, size_t capacity, size_t *length) {{\n  return {name}_encode(value, data, capacity, length) == WL_CODEC_OK ? WL_OK : WL_ERR_CORRUPT_PAYLOAD;\n}}\nwl_err_t {m}_kernel_send_{name}_view({m}_kernel_t *endpoint, const {name}_t *value) {{\n  if (endpoint == NULL || value == NULL) return WL_ERR_INVALID_ARG;\n  return wl_kernel_send(&endpoint->core, {}U, value, {m}_kernel_encode_{name}_view);\n}}", route.message_id).unwrap();
    }
    writeln!(source, "static wl_err_t {m}_kernel_route(wl_kernel_t *core, const wl_kernel_frame_t *frame) {{\n  {m}_kernel_t *endpoint = ({m}_kernel_t *)core;\n  (void)endpoint;\n  switch (frame->message_id) {{").unwrap();
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let req = type_name(&service.request_name);
        let resp = type_name(&service.response_name);
        writeln!(source, "  case {}U: return wl_kernel_response(core, frame->message_id, frame->payload, frame->length);\n  case {}U: {{\n    if (endpoint->handlers.on_rpc_{name} == NULL) return WL_ERR_NOT_SUPPORTED;\n    wl_kernel_rpc_metadata_t metadata;\n    wl_err_t error = wl_kernel_rpc_read(frame->payload, frame->length, 1, &metadata);\n    if (error != WL_OK) return error;\n    unsigned reply;\n    error = wl_kernel_reply_reserve(core, &reply);\n    if (error != WL_OK) return error;\n    {req}_t request;\n    if ({m}_rpc_{req}_decode(metadata.body, metadata.length, &request) != WL_CODEC_OK) {{\n      wl_kernel_reply_abort(core, reply);\n      return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    {m}_rpc_{resp}_value_clear(&endpoint->reply.{name});\n    int32_t status = endpoint->handlers.on_rpc_{name}(endpoint->user, &request, &endpoint->reply.{name});\n    return wl_kernel_reply_finish(core, reply, {}U, &metadata, status, &endpoint->reply.{name}, {m}_kernel_encode_{resp});\n  }}", service.response_id, service.request_id, service.response_id).unwrap();
    }
    for route in &p.direct_routes {
        let name = type_name(&route.message_name);
        writeln!(source, "  case {}U: {{\n    {name}_t message;\n    if ({name}_decode(frame->payload, frame->length, &message) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n    if (endpoint->handlers.on_message_{name} != NULL) endpoint->handlers.on_message_{name}(endpoint->user, &message);\n    return WL_OK;\n  }}", route.message_id).unwrap();
    }
    writeln!(source, "  default: return WL_ERR_NOT_SUPPORTED;\n  }}\n}}\nwl_err_t {m}_kernel_init({m}_kernel_t *endpoint, wl_packet_port_t *port, const {m}_kernel_handlers_t *handlers, wl_kernel_error_fn error, void *user) {{\n  if (endpoint == NULL || port == NULL || handlers == NULL) return WL_ERR_INVALID_ARG;\n  const {m}_kernel_handlers_t saved = *handlers;\n  memset(endpoint, 0, sizeof(*endpoint));\n  endpoint->handlers = saved;\n  endpoint->user = user;\n  wl_kernel_config_t config = {{.port = port, .route = {m}_kernel_route, .on_error = error, .user = user}};\n  for (unsigned i = 0U; i < WL_KERNEL_CALLS; ++i) config.tx_storage[i] = (wl_span_t){{endpoint->requests[i], sizeof(endpoint->requests[i])}};\n  for (unsigned i = 0U; i < WL_KERNEL_REPLIES; ++i) config.tx_storage[i + WL_KERNEL_CALLS] = (wl_span_t){{endpoint->replies[i], sizeof(endpoint->replies[i])}};\n  config.tx_storage[WL_KERNEL_TX_COUNT - 1U] = (wl_span_t){{endpoint->message, sizeof(endpoint->message)}};\n  return wl_kernel_init(&endpoint->core, &config);\n}}\nwl_err_t {m}_kernel_step({m}_kernel_t *endpoint, uint32_t now) {{\n  return endpoint == NULL ? WL_ERR_INVALID_ARG : wl_kernel_step(&endpoint->core, now, WL_KERNEL_BUDGET);\n}}").unwrap();
    let mut files = BTreeMap::from([
        (format!("{m}.h"), public.header),
        (format!("{m}_values.h"), public.values_header),
        (format!("{m}.c"), public.source),
        (format!("{m}_kernel.h"), header),
        (format!("{m}_kernel.c"), source),
        (format!("{m}_kernel_codecs.h"), private),
    ]);
    let mut deployment = format!(
        "{{\n  \"format\": \"wirelink-static-kernel-manifest-v1\",\n  \"kernel_contract\": 1,\n  \"profile_version\": 2,\n  \"envelope\": \"native_packet\",\n  \"integrity\": \"none\",\n  \"rpc_role\": \"both\",\n  \"call_slots\": 4,\n  \"reply_slots\": 4,\n  \"event_budget\": 16,\n  \"request_frame_capacity\": {req_max},\n  \"reply_frame_capacity\": {resp_max},\n  \"message_frame_capacity\": {msg_max},\n  \"messages\": [\n"
    );
    for (n, (id, (body, frame, codec))) in capacities.iter().enumerate() {
        writeln!(deployment, "    {{\"id\": {id}, \"codec\": \"{codec}\", \"body_max\": {body}, \"frame_max\": {frame}}}{}", if n + 1 == capacities.len() { "" } else { "," }).unwrap();
    }
    deployment.push_str("  ]\n}\n");
    files.insert(format!("{m}_kernel_deployment.json"), deployment);
    let artifacts = files
        .iter()
        .map(|(path, content)| crate::ManifestArtifact {
            path,
            contents: content.as_bytes(),
        })
        .collect::<Vec<_>>();
    let provenance = crate::generate_codegen_manifest(
        m,
        schema,
        Some(crate::binding_profile_identity(p)),
        &artifacts,
    );
    files.insert(format!("{m}_kernel_manifest.json"), provenance);
    Ok(GeneratedKernelC { files })
}

fn bounded(capacity: Option<u64>) -> Result<u64, KernelError> {
    match capacity {
        Some(n) if n <= 2048 => Ok(n),
        Some(_) => Err(KernelError(
            "selected message exceeds the native payload limit".into(),
        )),
        None => Err(KernelError(
            "selected messages need finite codec bounds for static storage".into(),
        )),
    }
}
