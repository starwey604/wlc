// SPDX-License-Identifier: Apache-2.0
//! Static routing over the compact C kernel; no old runtime code generation.
use crate::{
    KernelProfile, SemanticModel,
    codec_plan::CodecPlan,
    codegen::{plan::CModel, type_name, upper_snake},
    endpoint_layout::EndpointEnvelope,
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
    generate_kernel_c_with_pruning(schema, profile, module, true)
}

pub(crate) fn generate_kernel_c_with_pruning(
    schema: &SemanticModel,
    profile: &KernelProfile,
    module: &str,
    prune_routes: bool,
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
            "an endpoint deployment declaration is required".into(),
        ));
    }
    let envelope = p.endpoint.unwrap().envelope;
    let (wire_envelope, integrity, connector) = match envelope {
        EndpointEnvelope::NativePacket => ("native_packet", "none", None),
        EndpointEnvelope::CobsStream => ("cobs_stream", "crc16", Some("cobs_crc16")),
        EndpointEnvelope::BusLength16 => ("bus_length16", "none", Some("length16_none")),
        EndpointEnvelope::Any => return Err(KernelError("select an explicit deployment".into())),
    };
    let mut capacities = BTreeMap::new();
    let mut req_max = 12u64;
    let mut resp_max = 16u64;
    let mut msg_max = 4u64;
    for service in &p.rpc_services {
        for (id, response) in [(service.request_id, false), (service.response_id, true)] {
            let plan = CodecPlan::selected(schema, service.codec, [id])
                .map_err(|e| KernelError(e.to_string()))?;
            let body = bounded(plan.messages[&id].max_encoded_size)?;
            let frame = if response {
                (body + 12).max(16)
            } else {
                body + 12
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
        let plan = CodecPlan::selected(schema, route.codec, [route.message_id])
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
    let packet_max = capacities
        .values()
        .map(|(_, n, _)| *n)
        .max()
        .unwrap_or(4)
        .max(req_max)
        .max(resp_max)
        .max(msg_max);
    let wire_max = match envelope {
        EndpointEnvelope::CobsStream => packet_max + 2 + (packet_max + 2) / 254 + 2,
        EndpointEnvelope::BusLength16 => packet_max + 2,
        _ => packet_max,
    };
    let public = crate::codegen::generate_c_with_directions(schema, p, m, prune_routes)
        .map_err(|e| KernelError(e.to_string()))?;
    let mut header = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#ifndef {upper}_KERNEL_H\n#define {upper}_KERNEL_H\n#include \"{m}.h\"\n#include \"wirelink/kernel.h\"\n#if WL_KERNEL_CONTRACT != 1\n#error \"static kernel contract mismatch\"\n#endif\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n#define {upper}_KERNEL_PROFILE_VERSION 2U\n#define {upper}_KERNEL_PACKET_FRAME_MAX {packet_max}U\n#define {upper}_KERNEL_WIRE_FRAME_MAX {wire_max}U\n#define {upper}_KERNEL_REQUEST_FRAME_MAX {req_max}U\n#define {upper}_KERNEL_REPLY_FRAME_MAX {resp_max}U\n#define {upper}_KERNEL_MESSAGE_FRAME_MAX {msg_max}U\n"
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
            "  int32_t (*on_rpc_{name})(void *, const {req}_t *, {resp}_t *);"
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
            "    {}_t {};",
            type_name(&service.response_name),
            type_name(&service.name)
        )
        .unwrap();
    }
    writeln!(header, "  }} reply;\n  uint8_t requests[WL_KERNEL_CALLS][{req_max}U];\n  uint8_t replies[WL_KERNEL_REPLIES][{resp_max}U];\n  uint8_t message[{msg_max}U];\n}} {m}_kernel_t;\nwl_err_t {m}_kernel_init({m}_kernel_t *, wl_packet_port_t *, const {m}_kernel_handlers_t *, wl_kernel_error_fn, void *);\nwl_err_t {m}_kernel_step({m}_kernel_t *, uint32_t now);\n").unwrap();
    for service in &p.rpc_services {
        writeln!(header, "wl_err_t {m}_kernel_{}_start({m}_kernel_t *, const {}_t *, uint32_t now, uint32_t timeout, {m}_kernel_{}_callback_t, void *, wl_kernel_call_handle_t *);", type_name(&service.name), type_name(&service.request_name), type_name(&service.name)).unwrap();
    }
    for route in &p.send_routes {
        writeln!(
            header,
            "wl_err_t {m}_kernel_send_{}({m}_kernel_t *, const {}_t *);",
            type_name(&route.message_name),
            type_name(&route.message_name)
        )
        .unwrap();
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n#endif\n");
    let mut source = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#include \"{m}_kernel.h\"\n#include <string.h>\n"
    );
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let req = type_name(&service.request_name);
        let resp = type_name(&service.response_name);
        writeln!(
            source,
            "static void {m}_kernel_complete_{name}(wl_kernel_t *core, const wl_kernel_call_t *call, wl_kernel_result_t result, const uint8_t *body, size_t length) {{\n  (void)core;\n  {resp}_t response;\n  const {resp}_t *view = NULL;\n  if (result.outcome == WL_KERNEL_SUCCESS) {{\n    if ({resp}_decode(body, length, &response) == WL_CODEC_OK) view = &response;\n    else result = (wl_kernel_result_t){{WL_KERNEL_LOCAL_ERROR, WL_ERR_CORRUPT_PAYLOAD}};\n  }}\n  (({m}_kernel_{name}_callback_t)call->callback)(call->user, result, view);\n}}\nwl_err_t {m}_kernel_{name}_start({m}_kernel_t *endpoint, const {req}_t *request, uint32_t now, uint32_t timeout, {m}_kernel_{name}_callback_t callback, void *user, wl_kernel_call_handle_t *out) {{\n  if (out != NULL) *out = (wl_kernel_call_handle_t){{0}};\n  if (endpoint == NULL || request == NULL || callback == NULL || out == NULL || timeout == 0U || timeout > INT32_MAX) return WL_ERR_INVALID_ARG;\n  uint8_t slot;\n  uint8_t *payload;\n  wl_err_t error = wl_kernel_call_claim(&endpoint->core, &slot, &payload, NULL);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({req}_encode(request, payload, sizeof(endpoint->requests[0]) - WL_KERNEL_NATIVE_HEADER - WL_KERNEL_RPC_HEADER_DATA, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  return wl_kernel_call_commit(&endpoint->core, slot, {}U, {}U, length, now, timeout, {m}_kernel_complete_{name}, (void (*)(void))callback, user, out);\n}}",
            service.request_id, service.response_id
        )
        .unwrap();
    }
    for route in &p.send_routes {
        let name = type_name(&route.message_name);
        writeln!(
            source,
            "wl_err_t {m}_kernel_send_{name}({m}_kernel_t *endpoint, const {name}_t *value) {{\n  if (endpoint == NULL || value == NULL) return WL_ERR_INVALID_ARG;\n  uint8_t *frame;\n  wl_err_t error = wl_kernel_msg_claim(&endpoint->core, &frame, NULL);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({name}_encode(value, frame + WL_KERNEL_NATIVE_HEADER, sizeof(endpoint->message) - WL_KERNEL_NATIVE_HEADER, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  error = wl_wire_frame_seal(frame, sizeof(endpoint->message), WL_WIRE_MSG, {}U, length);\n  return error != WL_OK ? error : wl_kernel_msg_commit(&endpoint->core, WL_KERNEL_NATIVE_HEADER + length);\n}}",
            route.message_id
        )
        .unwrap();
    }
    writeln!(source, "static wl_err_t {m}_kernel_route(wl_kernel_t *core, const wl_kernel_frame_t *frame) {{\n  {m}_kernel_t *endpoint = ({m}_kernel_t *)core;\n  (void)endpoint;\n  switch (frame->message_id) {{").unwrap();
    for service in &p.rpc_services {
        let name = type_name(&service.name);
        let req = type_name(&service.request_name);
        let resp = type_name(&service.response_name);
        writeln!(
            source,
            "  case {}U: return wl_kernel_response(core, frame);\n  case {}U: {{\n    if (frame->kind != WL_WIRE_RPC_REQ) return WL_ERR_CORRUPT_PAYLOAD;\n    if (endpoint->handlers.on_rpc_{name} == NULL) return WL_ERR_NOT_SUPPORTED;\n    wl_kernel_rpc_metadata_t metadata;\n    wl_err_t error = wl_wire_rpc_read(frame->kind, frame->payload, frame->length, &metadata);\n    if (error != WL_OK) return error;\n    uint8_t reply;\n    uint8_t *payload;\n    error = wl_kernel_reply_claim(core, {}U, &metadata, &reply, &payload, NULL);\n    if (error != WL_OK) return error;\n    {req}_t request;\n    if ({req}_decode(metadata.body, metadata.length, &request) != WL_CODEC_OK) {{\n      wl_kernel_reply_abort(core, reply);\n      return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    {resp}_clear(&endpoint->reply.{name});\n    int32_t status = endpoint->handlers.on_rpc_{name}(endpoint->user, &request, &endpoint->reply.{name});\n    if (status != 0) return wl_kernel_reply_error(core, reply, status);\n    size_t length = 0U;\n    if ({resp}_encode(&endpoint->reply.{name}, payload, sizeof(endpoint->replies[0]) - WL_KERNEL_NATIVE_HEADER - WL_KERNEL_RPC_HEADER_DATA, &length) != WL_CODEC_OK) {{\n      wl_kernel_reply_abort(core, reply);\n      return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    return wl_kernel_reply_commit(core, reply, length);\n  }}",
            service.response_id, service.request_id, service.response_id
        )
        .unwrap();
    }
    for route in &p.direct_routes {
        let name = type_name(&route.message_name);
        writeln!(source, "  case {}U: {{\n    if (frame->kind != WL_WIRE_MSG) return WL_ERR_CORRUPT_PAYLOAD;\n    {name}_t message;\n    if ({name}_decode(frame->payload, frame->length, &message) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n    if (endpoint->handlers.on_message_{name} != NULL) endpoint->handlers.on_message_{name}(endpoint->user, &message);\n    return WL_OK;\n  }}", route.message_id).unwrap();
    }
    writeln!(source, "  default: return WL_ERR_NOT_SUPPORTED;\n  }}\n}}\nwl_err_t {m}_kernel_init({m}_kernel_t *endpoint, wl_packet_port_t *port, const {m}_kernel_handlers_t *handlers, wl_kernel_error_fn error, void *user) {{\n  if (endpoint == NULL || port == NULL || handlers == NULL) return WL_ERR_INVALID_ARG;\n  const {m}_kernel_handlers_t saved = *handlers;\n  memset(endpoint, 0, sizeof(*endpoint));\n  endpoint->handlers = saved;\n  endpoint->user = user;\n  wl_kernel_config_t config = {{\n      .port = port,\n      .storage = {{\n          .requests = &endpoint->requests[0][0],\n          .replies = &endpoint->replies[0][0],\n          .message = endpoint->message,\n          .request_stride = sizeof(endpoint->requests[0]),\n          .reply_stride = sizeof(endpoint->replies[0]),\n          .message_capacity = sizeof(endpoint->message),\n      }},\n      .route = {m}_kernel_route,\n      .on_error = error,\n      .user = user,\n  }};\n  return wl_kernel_init(&endpoint->core, &config);\n}}\nwl_err_t {m}_kernel_step({m}_kernel_t *endpoint, uint32_t now) {{\n  return endpoint == NULL ? WL_ERR_INVALID_ARG : wl_kernel_step(&endpoint->core, now, WL_KERNEL_BUDGET);\n}}").unwrap();
    let mut files = BTreeMap::from([
        (format!("{m}.h"), public.header),
        (format!("{m}.c"), format!("{}\n", public.source.trim_end())),
        (format!("{m}_kernel.h"), header),
        (format!("{m}_kernel.c"), source),
    ]);
    let mut deployment = format!(
        "{{\n  \"format\": \"wirelink-static-kernel-manifest-v1\",\n  \"kernel_contract\": 1,\n  \"profile_version\": 2,\n  \"envelope\": \"{wire_envelope}\",\n  \"integrity\": \"{integrity}\",\n  \"rpc_role\": \"both\",\n  \"call_slots\": 4,\n  \"reply_slots\": 3,\n  \"event_budget\": 16,\n  \"request_frame_capacity\": {req_max},\n  \"reply_frame_capacity\": {resp_max},\n  \"message_frame_capacity\": {msg_max},\n  \"messages\": [\n"
    );
    for (n, (id, (body, frame, codec))) in capacities.iter().enumerate() {
        writeln!(deployment, "    {{\"id\": {id}, \"codec\": \"{codec}\", \"body_max\": {body}, \"frame_max\": {frame}}}{}", if n + 1 == capacities.len() { "" } else { "," }).unwrap();
    }
    deployment.push_str("  ]\n}\n");
    files.insert(format!("{m}_kernel_deployment.json"), deployment);
    if let Some(connector) = connector {
        let transport_header = format!(
            r#"/* SPDX-License-Identifier: Apache-2.0 */
#ifndef {upper}_KERNEL_TRANSPORT_H
#define {upper}_KERNEL_TRANSPORT_H
#include "wirelink/packet_stream.h"
#ifdef __cplusplus
extern "C" {{
#endif
/* Stop the I/O producer and acknowledge physical drain before destroying
 * stream, port, RX or wire storage. Generated routing uses canonical DATA;
 * only this selected framing connector handles the external wire. */
wl_err_t {m}_kernel_transport_init(wl_packet_stream_t *, wl_packet_port_t *, uint32_t,
    wl_span_t rx, size_t rx_unit_size, wl_span_t wire, wl_packet_sink_fn,
    wl_packet_notify_fn, void *);
wl_err_t {m}_kernel_transport_feed(wl_packet_stream_t *, const uint8_t *, size_t, size_t *);
#ifdef __cplusplus
}}
#endif
#endif
"#
        );
        let transport_source = format!(
            r#"/* SPDX-License-Identifier: Apache-2.0 */
#include "{m}_kernel_transport.h"
#include "{m}_kernel.h"
#include <string.h>
wl_err_t {m}_kernel_transport_init(wl_packet_stream_t *stream, wl_packet_port_t *port,
    uint32_t instance, wl_span_t rx, size_t unit_size, wl_span_t wire,
    wl_packet_sink_fn submit, wl_packet_notify_fn notify, void *user) {{
  if (stream == NULL || port == NULL || submit == NULL ||
      wire.length < {wire_max}U || unit_size < {wire_max}U) return WL_ERR_INVALID_ARG;
  wl_packet_stream_config_t transport = {{.port = port, .tx_storage = wire,
      .physical_submit = submit, .notify = notify, .user = user,
      .maximum_frame_size = {packet_max}U}};
  memset(stream, 0, sizeof(*stream)); stream->config = transport;
  wl_packet_config_t config = {{.instance = instance, .rx_storage = rx.data,
      .rx_unit_size = unit_size, .rx_storage_size = rx.length,
      .submit = wl_packet_stream_{connector}_submit,
      .notify = wl_packet_stream_notify, .user = stream}};
  wl_err_t error = wl_packet_init(port, &config);
  if (error != WL_OK) return error;
  error = wl_packet_stream_{connector}_init(stream, &transport);
  if (error != WL_OK) wl_packet_stop(port);
  return error;
}}
wl_err_t {m}_kernel_transport_feed(wl_packet_stream_t *stream,
    const uint8_t *data, size_t length, size_t *consumed) {{
  return wl_packet_stream_{connector}_feed(stream, data, length, consumed);
}}
"#
        );
        files.insert(format!("{m}_kernel_transport.h"), transport_header);
        files.insert(format!("{m}_kernel_transport.c"), transport_source);
    }

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
