// SPDX-License-Identifier: Apache-2.0
//! Self-contained nodes over the allocation-free packet endpoint.
use crate::{
    ManifestArtifact, SemanticModel,
    ast::Cardinality,
    codec_plan::{CodecPlan, CodecProfile},
    codegen::{c_identifier, c_type, ordered_messages, type_name, upper_snake},
    node::{Framing, Handlers, Node, NodeError},
    semantic::{MessageSymbol, ResolvedType, Symbol},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedNodeC {
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, Default)]
struct Capacity {
    request: u64,
    reply: u64,
    message: u64,
    rx: u64,
    tx: u64,
    rx_wire: u64,
    tx_wire: u64,
}
fn body(model: &SemanticModel, id: u16, codec: CodecProfile) -> Result<u64, NodeError> {
    CodecPlan::selected(model, codec, [id])
        .map_err(|e| NodeError(e.to_string()))?
        .messages[&id]
        .max_encoded_size
        .filter(|n| *n <= 2048)
        .ok_or_else(|| NodeError("node messages need finite bounds <=2048".into()))
}
fn capacities(model: &SemanticModel, n: &Node) -> Result<Capacity, NodeError> {
    let mut c = Capacity::default();
    for r in &n.calls {
        c.request = c
            .request
            .max(body(model, r.request_id, CodecProfile::SchemaV1)? + 12);
        c.rx =
            c.rx.max((body(model, r.response_id, CodecProfile::SchemaV1)? + 12).max(16));
    }
    for r in &n.serves {
        c.reply = c
            .reply
            .max((body(model, r.response_id, CodecProfile::SchemaV1)? + 12).max(16));
        c.rx =
            c.rx.max(body(model, r.request_id, CodecProfile::SchemaV1)? + 12);
    }
    for r in &n.send_routes {
        c.message = c.message.max(body(model, r.message_id, r.codec)? + 4);
    }
    for r in &n.recv_routes {
        c.rx = c.rx.max(body(model, r.message_id, r.codec)? + 4);
    }
    c.tx = c.request.max(c.reply).max(c.message).max(4);
    c.rx = c.rx.max(4);
    if c.tx > 2052 || c.rx > 2052 {
        return Err(NodeError("node exceeds native frame limit 2052".into()));
    }
    c.rx_wire = n.framing.bound(c.rx);
    c.tx_wire = n.framing.bound(c.tx);
    Ok(c)
}
fn message(schema: &SemanticModel, id: u16) -> &MessageSymbol {
    schema
        .declarations
        .iter()
        .find_map(|s| match s {
            Symbol::Message(m) if m.id == id => Some(m),
            _ => None,
        })
        .expect("resolved message")
}
fn shortcut(m: &MessageSymbol) -> bool {
    (1..=4).contains(&m.fields.len())
        && m.fields.iter().all(|f| {
            f.cardinality == Cardinality::Required
                && !matches!(
                    f.ty,
                    ResolvedType::String | ResolvedType::Bytes | ResolvedType::Message { .. }
                )
        })
}
fn register(names: &mut BTreeSet<String>, name: String) -> Result<(), NodeError> {
    if !names.insert(name.clone()) {
        return Err(NodeError(format!(
            "generated C identifier collision `{name}`"
        )));
    }
    Ok(())
}
pub(crate) fn check_names(schema: &SemanticModel, nodes: &[Node]) -> Result<(), NodeError> {
    // C typedefs, functions and enum constants share the ordinary namespace.
    // Macro names are also included because they can replace any such token.
    let mut names = BTreeSet::new();
    for s in &schema.declarations {
        let name = type_name(s.name());
        register(&mut names, format!("{name}_t"))?;
        match s {
            Symbol::Enum(e) => {
                for v in &e.values {
                    register(&mut names, upper_snake(&v.name))?;
                }
            }
            Symbol::Message(m) => {
                for suffix in [
                    "clear",
                    "encoded_size",
                    "encode",
                    "decode",
                    "desc",
                    "fields",
                    "dense",
                    "wlc_clear_v",
                    "wlc_measure_v",
                    "wlc_encode_v",
                    "wlc_decode_v",
                    "pos_desc",
                ] {
                    register(&mut names, format!("{name}_{suffix}"))?;
                }
                for suffix in [
                    "MESSAGE_ID",
                    "CODEC_PROFILE",
                    "CODEC_FORMAT",
                    "HAS_MAX_ENCODED_SIZE",
                    "MAX_ENCODED_SIZE",
                ] {
                    register(&mut names, format!("{}_{suffix}", upper_snake(&m.name)))?;
                }
                for f in &m.fields {
                    if f.max_length.is_some()
                        && matches!(f.cardinality, Cardinality::Required | Cardinality::Optional)
                    {
                        register(&mut names, format!("{name}_set_{}", c_identifier(&f.name)))?;
                    }
                }
            }
        }
    }
    for n in nodes {
        let mut reply_members = BTreeSet::new();
        let mut handler_members = BTreeSet::new();
        for r in &n.serves {
            register(&mut reply_members, type_name(&r.name))?;
            if n.handlers == Handlers::Dynamic {
                register(&mut handler_members, format!("on_{}", type_name(&r.name)))?;
            }
        }
        if n.handlers == Handlers::Dynamic {
            for r in &n.recv_routes {
                register(
                    &mut handler_members,
                    format!("on_{}", type_name(&r.message_name)),
                )?;
            }
        }
        let p = &n.name;
        for suffix in [
            "REQUEST_FRAME_MAX",
            "REPLY_FRAME_MAX",
            "MESSAGE_FRAME_MAX",
            "RX_FRAME_MAX",
            "TX_FRAME_MAX",
            "RX_WIRE_MAX",
            "TX_WIRE_MAX",
        ] {
            register(&mut names, format!("{}_{suffix}", upper_snake(p)))?;
        }
        for suffix in [
            "node_t",
            "bind",
            "bind_instance",
            "bind_sync",
            "bind_sync_instance",
            "bind_impl",
            "route",
        ] {
            register(&mut names, format!("{p}_{suffix}"))?;
        }
        if n.handlers == Handlers::Dynamic {
            register(&mut names, format!("{p}_handlers_t"))?;
        }
        if n.framing != Framing::Native {
            register(&mut names, format!("{p}_stream_ops"))?;
        }
        if !n.calls.is_empty() || !n.serves.is_empty() || !n.send_routes.is_empty() {
            for suffix in ["tx_write_fn", "sync_write"] {
                register(&mut names, format!("{p}_{suffix}"))?;
            }
        }
        for r in &n.send_routes {
            let m = type_name(&r.message_name);
            register(&mut names, format!("{p}_send_{m}"))?;
            if shortcut(message(schema, r.message_id)) {
                register(&mut names, format!("{p}_send_{m}_val"))?;
            }
        }
        for r in &n.recv_routes {
            if n.handlers == Handlers::Static {
                register(
                    &mut names,
                    r.handler
                        .clone()
                        .unwrap_or_else(|| format!("{p}_on_{}", type_name(&r.message_name))),
                )?;
            }
        }
        for r in &n.serves {
            if n.handlers == Handlers::Static {
                register(
                    &mut names,
                    r.handler
                        .clone()
                        .unwrap_or_else(|| format!("{p}_on_{}", type_name(&r.name))),
                )?;
            }
        }
        for r in &n.calls {
            let rpc = type_name(&r.name);
            for suffix in [
                format!("{rpc}_cb_t"),
                format!("{rpc}_ex_cb_t"),
                format!("call_{rpc}"),
                format!("call_{rpc}_ex"),
                format!("call_{rpc}_start"),
                format!("complete_{rpc}"),
                format!("complete_{rpc}_ex"),
            ] {
                register(&mut names, format!("{p}_{suffix}"))?;
            }
            if let Some(h) = &r.handler {
                register(&mut names, h.clone())?;
            }
        }
    }
    Ok(())
}
pub fn generate_node_c(
    schema: &SemanticModel,
    nodes: &[Node],
    module: &str,
) -> Result<GeneratedNodeC, NodeError> {
    generate_node_c_with_pruning(schema, nodes, module, true)
}
pub fn generate_node_c_with_pruning(
    schema: &SemanticModel,
    nodes: &[Node],
    module: &str,
    prune: bool,
) -> Result<GeneratedNodeC, NodeError> {
    check_names(schema, nodes)?;
    let module = c_identifier(module);
    let mut c = if nodes.is_empty() {
        crate::generate_c(schema, &module)
    } else {
        crate::generate_c_for_nodes(schema, nodes, &module, prune)
    }
    .map_err(|e| NodeError(e.to_string()))?;
    let ending = "#ifdef __cplusplus\n}\n#endif\n\n#endif\n";
    c.header = c
        .header
        .strip_suffix(ending)
        .expect("codec header suffix")
        .to_owned();
    if !nodes.is_empty() {
        c.header.push_str(
            "#include <wirelink/node.h>\n#ifndef __cplusplus\n#include <stdalign.h>\n#endif\n",
        );
    }
    writeln!(c.header, "#include <wirelink/version.h>\n#if WIRELINK_CODEGEN_CONTRACT != {0}\n#error \"WLC requires generated-code contract {0}\"\n#endif", crate::CODEGEN_CONTRACT).unwrap();
    c.source.push_str("\n#include <limits.h>\n");
    // A Node accepts only finite routes. Bounded bytes may also live in a
    // nested message; positional-v1 does not admit bytes or nested messages.
    let mut local = BTreeSet::new();
    for m in ordered_messages(schema).map_err(|e| NodeError(e.to_string()))? {
        if m.fields.iter().any(|f| match f.ty {
            ResolvedType::Bytes => f.max_length.is_some(),
            ResolvedType::Message { id, .. } => local.contains(&id),
            _ => false,
        }) {
            local.insert(m.id);
        }
    }
    let incoming: BTreeSet<_> = nodes
        .iter()
        .flat_map(|n| {
            n.recv_routes
                .iter()
                .map(|r| r.message_id)
                .chain(n.serves.iter().map(|r| r.request_id))
                .chain(n.calls.iter().map(|r| r.response_id))
        })
        .collect();
    local.retain(|id| incoming.contains(id));
    if !local.is_empty() {
        // Older contract-1 headers still provide the transactional decoder.
        c.source.push_str("#ifdef WL_CODEC_HAS_LOCAL_DECODE\n#define WLC_NODE_DECODE(m, ...) wl_codec_decode_local(&m##_desc, __VA_ARGS__)\n#else\n#define WLC_NODE_DECODE(m, ...) m##_decode(__VA_ARGS__)\n#endif\n");
    }
    let mut metadata = Vec::new();
    for n in nodes {
        let cap = capacities(schema, n)?;
        emit_node(schema, n, cap, &local, &mut c.header, &mut c.source);
        metadata.push(node_metadata(n, cap));
    }
    c.header.push_str(ending);
    let mut files = BTreeMap::from([
        (format!("{module}.h"), c.header),
        (format!("{module}.c"), c.source),
    ]);
    let artifacts = files
        .iter()
        .map(|(path, c)| ManifestArtifact {
            path,
            contents: c.as_bytes(),
        })
        .collect::<Vec<_>>();
    let mut manifest = crate::generate_codegen_manifest(
        &module,
        schema,
        if nodes.len() == 1 {
            Some(crate::node_identity(&nodes[0]))
        } else {
            None
        },
        &artifacts,
    );
    let end = manifest.rfind('}').expect("manifest object");
    manifest.insert_str(end, &format!(",\n  \"nodes\": [{}]\n", metadata.join(",")));
    files.insert(format!("{module}_manifest.json"), manifest);
    Ok(GeneratedNodeC { files })
}
fn node_metadata(n: &Node, c: Capacity) -> String {
    format!(
        "{{\"name\":\"{}\",\"framing\":\"{}\",\"handlers\":\"{}\",\"timeout_ms\":{},\"rx_slots\":{},\"request_frame_capacity\":{},\"reply_frame_capacity\":{},\"message_frame_capacity\":{},\"rx_frame_capacity\":{},\"tx_frame_capacity\":{},\"rx_wire_capacity\":{},\"tx_wire_capacity\":{}}}",
        n.name,
        n.framing.name(),
        n.handlers.name(),
        n.timeout_ms,
        n.rx_slots,
        c.request,
        c.reply,
        c.message,
        c.rx,
        c.tx,
        c.rx_wire,
        c.tx_wire
    )
}
fn decode_call(local: &BTreeSet<u16>, id: u16, name: &str, args: &str) -> String {
    if local.contains(&id) {
        format!("WLC_NODE_DECODE({name}, {args})")
    } else {
        format!("{name}_decode({args})")
    }
}
fn emit_node(
    schema: &SemanticModel,
    n: &Node,
    c: Capacity,
    local: &BTreeSet<u16>,
    h: &mut String,
    s: &mut String,
) {
    let p = &n.name;
    let upper = upper_snake(p);
    let request_stride = (c.request + 3) & !3;
    let reply_stride = (c.reply + 3) & !3;
    let dynamic = n.handlers == Handlers::Dynamic;
    let stream = n.framing != Framing::Native;
    let tx = !n.calls.is_empty() || !n.serves.is_empty() || !n.send_routes.is_empty();
    let connector = if n.framing == Framing::CobsCrc16 {
        "cobs_crc16"
    } else {
        "length16_none"
    };
    writeln!(h,"\ntypedef struct {p}_node {p}_node_t;\n#define {upper}_REQUEST_FRAME_MAX {}U\n#define {upper}_REPLY_FRAME_MAX {}U\n#define {upper}_MESSAGE_FRAME_MAX {}U\n#define {upper}_RX_FRAME_MAX {}U\n#define {upper}_TX_FRAME_MAX {}U\n#define {upper}_RX_WIRE_MAX {}U\n#define {upper}_TX_WIRE_MAX {}U",c.request,c.reply,c.message,c.rx,c.tx,c.rx_wire,c.tx_wire).unwrap();
    if tx {
        writeln!(h, "/* Accept and drain the whole frame, or reject it without consuming bytes. */\ntypedef wl_err_t (*{p}_tx_write_fn)({p}_node_t *self, const uint8_t *data, size_t length);").unwrap();
    }
    if dynamic {
        writeln!(h, "typedef struct {{").unwrap();
        if n.serves.is_empty() && n.recv_routes.is_empty() {
            h.push_str("  uint8_t _empty;\n");
        }
        for r in &n.serves {
            writeln!(
                h,
                "  int32_t (*on_{})(void *, const {}_t *, {}_t *);",
                type_name(&r.name),
                type_name(&r.request_name),
                type_name(&r.response_name)
            )
            .unwrap();
        }
        for r in &n.recv_routes {
            let m = type_name(&r.message_name);
            writeln!(h, "  void (*on_{m})(void *, const {m}_t *);").unwrap();
        }
        writeln!(h, "}} {p}_handlers_t;").unwrap();
    }
    writeln!(h, "struct {p}_node {{\n  wl_node_t node;").unwrap();
    if dynamic {
        writeln!(h, "  {p}_handlers_t handlers;").unwrap();
    }
    if !n.serves.is_empty() {
        h.push_str("  union {\n");
        for r in &n.serves {
            writeln!(
                h,
                "    {}_t {};",
                type_name(&r.response_name),
                type_name(&r.name)
            )
            .unwrap();
        }
        writeln!(
            h,
            "  }} reply;\n  alignas(4) uint8_t replies[WL_ENDPOINT_REPLIES][{reply_stride}];"
        )
        .unwrap();
    }
    if !n.calls.is_empty() {
        writeln!(
            h,
            "  alignas(4) uint8_t requests[WL_ENDPOINT_CALLS][{request_stride}];"
        )
        .unwrap();
    }
    if !n.send_routes.is_empty() {
        writeln!(h, "  alignas(4) uint8_t message[{}];", c.message).unwrap();
    }
    // Default packet-port RX slots are indexed by rx_wire_max + 1, not a padded stride.
    writeln!(
        h,
        "  alignas(4) uint8_t rx[{}U][{upper}_RX_WIRE_MAX + 1U];",
        n.rx_slots
    )
    .unwrap();
    if tx {
        writeln!(h, "  {p}_tx_write_fn tx_write;").unwrap();
    }
    if stream {
        writeln!(
            h,
            "  wl_packet_stream_t stream;\n  uint8_t tx_wire[{}];",
            c.tx_wire
        )
        .unwrap();
    }
    h.push_str("};\n");
    let extra = if dynamic {
        format!(", const {p}_handlers_t *handlers, void *user")
    } else {
        String::new()
    };
    let tx_arg = if tx {
        format!(", {p}_tx_write_fn tx_write")
    } else {
        String::new()
    };
    let args = if dynamic { ", handlers, user" } else { "" };
    writeln!(
        h,
        "/* Explicit instance IDs must be nonzero. Rebind only after close completes. */
wl_err_t {p}_bind({p}_node_t *self, wl_transport_t *transport{extra});
wl_err_t {p}_bind_instance({p}_node_t *self, uint32_t instance, wl_transport_t *transport{extra});
wl_err_t {p}_bind_sync({p}_node_t *self{tx_arg}{extra});
wl_err_t {p}_bind_sync_instance({p}_node_t *self, uint32_t instance{tx_arg}{extra});"
    )
    .unwrap();
    for r in &n.recv_routes {
        if !dynamic {
            writeln!(
                h,
                "void {}({p}_node_t *self, const {}_t *message);",
                r.handler
                    .clone()
                    .unwrap_or_else(|| format!("{p}_on_{}", type_name(&r.message_name))),
                type_name(&r.message_name)
            )
            .unwrap();
        }
    }
    for r in &n.serves {
        if !dynamic {
            writeln!(
                h,
                "int32_t {}({p}_node_t *self, const {}_t *request, {}_t *response);",
                r.handler
                    .clone()
                    .unwrap_or_else(|| format!("{p}_on_{}", type_name(&r.name))),
                type_name(&r.request_name),
                type_name(&r.response_name)
            )
            .unwrap();
        }
    }
    for r in &n.send_routes {
        let m = type_name(&r.message_name);
        writeln!(
            h,
            "wl_err_t {p}_send_{m}({p}_node_t *self, const {m}_t *message);"
        )
        .unwrap();
        let msg = message(schema, r.message_id);
        if shortcut(msg) {
            let args = msg
                .fields
                .iter()
                .enumerate()
                .map(|(i, f)| format!(", {} arg{i}", c_type(&f.ty)))
                .collect::<String>();
            writeln!(
                h,
                "static inline wl_err_t {p}_send_{m}_val({p}_node_t *self{args}) {{\n  {m}_t value;"
            )
            .unwrap();
            for (i, f) in msg.fields.iter().enumerate() {
                let id = c_identifier(&f.name);
                writeln!(h, "  value.{id} = arg{i};").unwrap();
            }
            writeln!(h, "  return {p}_send_{m}(self, &value);\n}}").unwrap();
        }
        writeln!(s,"wl_err_t {p}_send_{m}({p}_node_t *self, const {m}_t *message) {{\n  if (self == NULL || message == NULL) return WL_ERR_INVALID_ARG;\n  uint8_t *frame; size_t capacity;\n  wl_err_t error = wl_endpoint_msg_claim(&self->node.core, &frame, &capacity);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({m}_encode(message, frame + 4U, capacity - 4U, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  error = wl_wire_frame_seal(frame, capacity, WL_WIRE_MSG, {}U, length);\n  return error == WL_OK ? wl_endpoint_msg_commit(&self->node.core, length + 4U) : error;\n}}",r.message_id).unwrap();
    }
    emit_calls(n, local, h, s);
    let has_route = !n.calls.is_empty() || !n.serves.is_empty() || !n.recv_routes.is_empty();
    if has_route {
        emit_route(n, local, s);
    }
    if tx {
        writeln!(
            s,
            "static wl_err_t {p}_sync_write(wl_node_t *node, const uint8_t *data, size_t length) {{
  {p}_node_t *self = ({p}_node_t *)node;
  return self->tx_write(self, data, length);
}}"
        )
        .unwrap();
    }
    if stream {
        let framing = if n.framing == Framing::CobsCrc16 {
            "WL_NODE_COBS_CRC16"
        } else {
            "WL_NODE_LENGTH16"
        };
        writeln!(
            s,
            "static const wl_node_stream_ops_t {p}_stream_ops = {{
  .init = wl_packet_stream_{connector}_init,
  .submit = wl_packet_stream_{connector}_submit,
  .feed = wl_packet_stream_{connector}_feed,
  .abort_rx = wl_packet_stream_abort_rx,
  .notify = wl_packet_stream_notify,
  .framing = {framing}
}};"
        )
        .unwrap();
    }
    writeln!(s, "static wl_err_t {p}_bind_impl({p}_node_t *self, uint32_t instance, wl_transport_t *transport{tx_arg}{extra}) {{
  if (self == NULL) return WL_ERR_INVALID_ARG;").unwrap();
    if tx {
        s.push_str("  if (transport == NULL && tx_write == NULL) return WL_ERR_INVALID_ARG;\n");
    }
    if dynamic {
        s.push_str(
            "  if (handlers == NULL) return WL_ERR_INVALID_ARG;\n  self->handlers = *handlers;\n",
        );
    }
    if tx {
        s.push_str("  self->tx_write = tx_write;\n");
    }
    writeln!(
        s,
        "  const wl_node_layout_t layout = {{
    .instance = instance, .has_tx = {}, .rx_slots = {}U,
    .route = {}, .user = {},
    .rx_storage = &self->rx[0][0], .rx_storage_size = sizeof(self->rx),
    .rx_frame_max = {upper}_RX_FRAME_MAX, .tx_frame_max = {upper}_TX_FRAME_MAX,
    .rx_wire_max = {upper}_RX_WIRE_MAX, .tx_wire_max = {upper}_TX_WIRE_MAX,",
        u8::from(tx),
        n.rx_slots,
        if has_route {
            format!("{p}_route")
        } else {
            "NULL".into()
        },
        if dynamic { "user" } else { "NULL" }
    )
    .unwrap();
    if stream {
        writeln!(
            s,
            "    .stream = &self->stream, .stream_ops = &{p}_stream_ops,
    .stream_tx_wire = {{self->tx_wire, sizeof(self->tx_wire)}},"
        )
        .unwrap();
    }
    s.push_str("    .storage = {\n");
    for (field, stride, cap, array) in [
        ("requests", "request_stride", request_stride, true),
        ("replies", "reply_stride", reply_stride, true),
        ("message", "message_capacity", c.message, false),
    ] {
        writeln!(
            s,
            "          .{field} = {}, .{stride} = {cap}U,",
            if cap == 0 {
                "NULL".into()
            } else if array {
                format!("&self->{field}[0][0]")
            } else {
                format!("self->{field}")
            }
        )
        .unwrap();
    }
    let sync_write = if tx {
        format!("{p}_sync_write")
    } else {
        "NULL".into()
    };
    writeln!(
        s,
        "    }}
  }};
  return transport != NULL ? wl_node_bind_transport(&self->node, &layout, transport)
                           : wl_node_bind_sync(&self->node, &layout, {sync_write});
}}"
    )
    .unwrap();
    for synchronous in [false, true] {
        let suffix = if synchronous { "_sync" } else { "" };
        let param = if synchronous {
            tx_arg.as_str()
        } else {
            ", wl_transport_t *transport"
        };
        let value = if synchronous { "NULL" } else { "transport" };
        let write_arg = if !tx {
            ""
        } else if synchronous {
            ", tx_write"
        } else {
            ", NULL"
        };
        for explicit in [false, true] {
            let instance_param = if explicit { ", uint32_t instance" } else { "" };
            let instance = if explicit { "instance" } else { "0U" };
            let name = if explicit { "_instance" } else { "" };
            writeln!(
                s,
                "wl_err_t {p}_bind{suffix}{name}({p}_node_t *self{instance_param}{param}{extra}) {{"
            )
            .unwrap();
            if explicit {
                s.push_str("  if (instance == 0U) return WL_ERR_INVALID_ARG;\n");
            }
            if !synchronous {
                s.push_str("  if (transport == NULL) return WL_ERR_INVALID_ARG;\n");
            }
            writeln!(
                s,
                "  return {p}_bind_impl(self, {instance}, {value}{write_arg}{args});\n}}"
            )
            .unwrap();
        }
    }
}
fn emit_calls(n: &Node, local: &BTreeSet<u16>, h: &mut String, s: &mut String) {
    let p = &n.name;
    for r in &n.calls {
        let rpc = type_name(&r.name);
        let req = type_name(&r.request_name);
        let resp = type_name(&r.response_name);
        writeln!(h,"typedef void (*{p}_{rpc}_cb_t)({p}_node_t *, wl_rpc_result_t, const {resp}_t *);\ntypedef void (*{p}_{rpc}_ex_cb_t)(void *, wl_rpc_result_t, const {resp}_t *);").unwrap();
        if let Some(done) = &r.handler {
            writeln!(
                h,
                "void {done}({p}_node_t *self, wl_rpc_result_t result, const {resp}_t *response);"
            )
            .unwrap();
        }
        let cb = if r.handler.is_none() {
            format!(", {p}_{rpc}_cb_t callback")
        } else {
            String::new()
        };
        writeln!(h,"/* Simplified calls start their timeout on the next step(now). */\nwl_err_t {p}_call_{rpc}({p}_node_t *self, const {req}_t *request{cb});\nwl_err_t {p}_call_{rpc}_ex({p}_node_t *self, const {req}_t *request, uint32_t now, uint32_t timeout_ms, {p}_{rpc}_ex_cb_t callback, void *user, wl_call_handle_t *out);").unwrap();
        let decode = decode_call(local, r.response_id, &resp, "body, length, &response");
        for ex in [false, true] {
            let suffix = if ex { "_ex" } else { "" };
            let ctx = if ex {
                "call->user"
            } else {
                &format!("({p}_node_t *)core")
            };
            writeln!(s,"static void {p}_complete_{rpc}{suffix}(wl_endpoint_t *core, const wl_endpoint_call_t *call, wl_rpc_result_t result, const uint8_t *body, size_t length) {{\n  (void)core;\n  {resp}_t response;\n  const {resp}_t *view = NULL;\n  if (result.outcome == WL_RPC_SUCCESS) {{\n    if ({decode} == WL_CODEC_OK) view = &response;\n    else result = (wl_rpc_result_t){{WL_RPC_LOCAL_ERROR, WL_ERR_CORRUPT_PAYLOAD}};\n  }}\n  (({p}_{rpc}{suffix}_cb_t)call->callback)({ctx}, result, view);\n}}").unwrap();
        }
        writeln!(s,"static wl_err_t {p}_call_{rpc}_start({p}_node_t *self, const {req}_t *request, uint32_t now, uint32_t timeout, uint8_t has_now, wl_endpoint_complete_fn complete, void (*callback)(void), void *user, wl_call_handle_t *out) {{\n  if (out != NULL) *out = (wl_call_handle_t){{0}};\n  if (self == NULL || request == NULL || callback == NULL || timeout > INT32_MAX) return WL_ERR_INVALID_ARG;\n  if (timeout == 0U) timeout = {}U;\n  uint8_t slot; uint8_t *payload; size_t capacity;\n  wl_err_t error = wl_endpoint_call_claim(&self->node.core, &slot, &payload, &capacity);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({req}_encode(request, payload, capacity, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  return wl_endpoint_call_commit(&self->node.core, slot, {}U, {}U, length, now, timeout, has_now, complete, callback, user, out);\n}}",n.timeout_ms,r.request_id,r.response_id).unwrap();
        let callback = r.handler.as_deref().unwrap_or("callback");
        writeln!(s,"wl_err_t {p}_call_{rpc}({p}_node_t *self, const {req}_t *request{cb}) {{\n  return {p}_call_{rpc}_start(self, request, 0U, 0U, 0U, {p}_complete_{rpc}, (void (*)(void)){callback}, self, NULL);\n}}\nwl_err_t {p}_call_{rpc}_ex({p}_node_t *self, const {req}_t *request, uint32_t now, uint32_t timeout_ms, {p}_{rpc}_ex_cb_t callback, void *user, wl_call_handle_t *out) {{\n  return {p}_call_{rpc}_start(self, request, now, timeout_ms, 1U, {p}_complete_{rpc}_ex, (void (*)(void))callback, user, out);\n}}").unwrap();
    }
}
fn emit_route(n: &Node, local: &BTreeSet<u16>, s: &mut String) {
    let p = &n.name;
    let dynamic = n.handlers == Handlers::Dynamic;
    writeln!(s,"static wl_err_t {p}_route(wl_endpoint_t *core, const wl_wire_frame_t *frame) {{\n  {p}_node_t *self = ({p}_node_t *)core;\n  (void)self;\n  switch (frame->message_id) {{").unwrap();
    for r in &n.calls {
        writeln!(
            s,
            "  case {}U: return wl_endpoint_response(core, frame);",
            r.response_id
        )
        .unwrap();
    }
    for r in &n.serves {
        let rpc = type_name(&r.name);
        let req = type_name(&r.request_name);
        let resp = type_name(&r.response_name);
        writeln!(s,"  case {}U: {{\n    if (frame->kind != WL_WIRE_RPC_REQ) return WL_ERR_CORRUPT_PAYLOAD;",r.request_id).unwrap();
        if dynamic {
            writeln!(
                s,
                "    if (self->handlers.on_{rpc} == NULL) return WL_ERR_NOT_SUPPORTED;"
            )
            .unwrap();
        }
        let decode = decode_call(
            local,
            r.request_id,
            &req,
            "metadata.body, metadata.length, &request",
        );
        writeln!(s,"    wl_rpc_metadata_t metadata;\n    wl_err_t error = wl_wire_rpc_read(frame->kind, frame->payload, frame->length, &metadata);\n    if (error != WL_OK) return error;\n    uint8_t slot; uint8_t *payload; size_t capacity;\n    error = wl_endpoint_reply_claim(core, {}U, &metadata, &slot, &payload, &capacity);\n    if (error != WL_OK) return error;\n    {req}_t request;\n    if ({decode} != WL_CODEC_OK) {{\n      wl_endpoint_reply_abort(core, slot); return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    {resp}_clear(&self->reply.{rpc});",r.response_id).unwrap();
        let handler = if dynamic {
            format!("self->handlers.on_{rpc}")
        } else {
            r.handler.clone().unwrap_or_else(|| format!("{p}_on_{rpc}"))
        };
        let user = if dynamic { "self->node.user" } else { "self" };
        writeln!(s,"    int32_t status = {handler}({user}, &request, &self->reply.{rpc});\n    if (status != 0) return wl_endpoint_reply_error(core, slot, status);\n    size_t length = 0U;\n    if ({resp}_encode(&self->reply.{rpc}, payload, capacity, &length) != WL_CODEC_OK) {{\n      wl_endpoint_reply_abort(core, slot); return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    return wl_endpoint_reply_commit(core, slot, length);\n  }}").unwrap();
    }
    for r in &n.recv_routes {
        let m = type_name(&r.message_name);
        let handler = if dynamic {
            format!("self->handlers.on_{m}")
        } else {
            r.handler.clone().unwrap_or_else(|| format!("{p}_on_{m}"))
        };
        let user = if dynamic { "self->node.user" } else { "self" };
        let decode = decode_call(
            local,
            r.message_id,
            &m,
            "frame->payload, frame->length, &message",
        );
        writeln!(s,"  case {}U: {{\n    if (frame->kind != WL_WIRE_MSG) return WL_ERR_CORRUPT_PAYLOAD;\n    {m}_t message;\n    if ({decode} != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;",r.message_id).unwrap();
        writeln!(
            s,
            "    {}{handler}({user}, &message);\n    return WL_OK;\n  }}",
            if dynamic {
                format!("if ({handler} != NULL) ")
            } else {
                String::new()
            }
        )
        .unwrap();
    }
    s.push_str("  default: return WL_ERR_NOT_SUPPORTED;\n  }\n}\n");
}
