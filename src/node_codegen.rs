// SPDX-License-Identifier: Apache-2.0
//! Self-contained nodes over the allocation-free packet endpoint.
use crate::{
    ManifestArtifact, SemanticModel,
    ast::Cardinality,
    codec_plan::{CodecPlan, CodecProfile},
    codegen::{c_identifier, c_type, type_name, upper_snake},
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
            "init",
            "step",
            "close",
            "quiescent",
            "route",
            "error",
        ] {
            register(&mut names, format!("{p}_{suffix}"))?;
        }
        if n.handlers == Handlers::Dynamic {
            register(&mut names, format!("{p}_handlers_t"))?;
        }
        for suffix in ["init_with_instance", "init_io", "submit", "tx_ready"] {
            register(&mut names, format!("{p}_{suffix}"))?;
        }
        let rx = if n.framing == Framing::Native {
            "rx_packet"
        } else {
            "rx_feed"
        };
        register(&mut names, format!("{p}_{rx}"))?;
        if !n.calls.is_empty() || !n.serves.is_empty() || !n.send_routes.is_empty() {
            for suffix in ["tx_write_fn", "init_sync"] {
                register(&mut names, format!("{p}_{suffix}"))?;
            }
        }
        if !n.calls.is_empty() {
            register(&mut names, format!("{p}_cancel"))?;
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
        c.header.push_str("#include <wirelink/endpoint.h>\n");
    }
    writeln!(c.header, "#include <wirelink/version.h>\n#if WIRELINK_CODEGEN_CONTRACT != {0}\n#error \"WLC requires generated-code contract {0}\"\n#endif", crate::CODEGEN_CONTRACT).unwrap();
    if nodes.iter().any(|n| n.framing != Framing::Native) {
        c.header.push_str("#include <wirelink/packet_stream.h>\n");
    }
    c.source.push_str("\n#include <limits.h>\n");
    let mut metadata = Vec::new();
    for n in nodes {
        let cap = capacities(schema, n)?;
        emit_node(schema, n, cap, &mut c.header, &mut c.source);
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
        "{{\"name\":\"{}\",\"framing\":\"{}\",\"handlers\":\"{}\",\"timeout_ms\":{},\"request_frame_capacity\":{},\"reply_frame_capacity\":{},\"message_frame_capacity\":{},\"rx_frame_capacity\":{},\"tx_frame_capacity\":{},\"rx_wire_capacity\":{},\"tx_wire_capacity\":{}}}",
        n.name,
        n.framing.name(),
        n.handlers.name(),
        n.timeout_ms,
        c.request,
        c.reply,
        c.message,
        c.rx,
        c.tx,
        c.rx_wire,
        c.tx_wire
    )
}
fn emit_node(schema: &SemanticModel, n: &Node, c: Capacity, h: &mut String, s: &mut String) {
    let p = &n.name;
    let upper = upper_snake(p);
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
    writeln!(
        h,
        "struct {p}_node {{\n  wl_endpoint_t core;\n  void *user;\n  wl_endpoint_error_fn on_error;"
    )
    .unwrap();
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
            "  }} reply;\n  uint8_t replies[WL_ENDPOINT_REPLIES][{}];",
            c.reply
        )
        .unwrap();
    }
    if !n.calls.is_empty() {
        writeln!(h, "  uint8_t requests[WL_ENDPOINT_CALLS][{}];", c.request).unwrap();
    }
    if !n.send_routes.is_empty() {
        writeln!(h, "  uint8_t message[{}];", c.message).unwrap();
    }
    writeln!(
        h,
        "  wl_packet_port_t port;\n  uint8_t rx[WL_PACKET_RX_SLOTS][{upper}_RX_WIRE_MAX + 1U];"
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
    writeln!(h,"/* Automatic instances are unique only within this process lifetime.\n * Stop I/O before reinitialization; persistent links should supply an instance. */\nwl_err_t {p}_init({p}_node_t *self{tx_arg}{extra});\nwl_err_t {p}_init_with_instance({p}_node_t *self, uint32_t instance{tx_arg}{extra});\n/* Instance zero requests an automatic instance. */\nwl_err_t {p}_init_io({p}_node_t *self, uint32_t instance, const wl_packet_io_t *io{extra});").unwrap();
    writeln!(
        h,
        "static inline void {p}_tx_ready({p}_node_t *self) {{ wl_packet_activity(&self->port); }}"
    )
    .unwrap();
    if stream {
        writeln!(h,"wl_err_t {p}_rx_feed({p}_node_t *self, const uint8_t *data, size_t length, size_t *consumed);").unwrap();
    } else {
        writeln!(
            h,
            "wl_err_t {p}_rx_packet({p}_node_t *self, const uint8_t *data, size_t length);"
        )
        .unwrap();
    }
    writeln!(h,"wl_err_t {p}_step({p}_node_t *self, uint32_t now);\n/* Stop the RX producer before close, then step until quiescent. */\nwl_err_t {p}_close({p}_node_t *self);\nint {p}_quiescent(const {p}_node_t *self);").unwrap();
    if !n.calls.is_empty() {
        writeln!(
            h,
            "wl_err_t {p}_cancel({p}_node_t *self, wl_call_handle_t handle);"
        )
        .unwrap();
    }
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
        writeln!(s,"wl_err_t {p}_send_{m}({p}_node_t *self, const {m}_t *message) {{\n  if (self == NULL || message == NULL) return WL_ERR_INVALID_ARG;\n  uint8_t *frame; size_t capacity;\n  wl_err_t error = wl_endpoint_msg_claim(&self->core, &frame, &capacity);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({m}_encode(message, frame + 4U, capacity - 4U, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  error = wl_wire_frame_seal(frame, capacity, WL_WIRE_MSG, {}U, length);\n  return error == WL_OK ? wl_endpoint_msg_commit(&self->core, length + 4U) : error;\n}}",r.message_id).unwrap();
    }
    emit_calls(n, h, s);
    writeln!(s,"static void {p}_error(void *user, uint16_t id, wl_err_t error) {{\n  {p}_node_t *self = user;\n  if (self->on_error != NULL) self->on_error(self->user, id, error);\n}}").unwrap();
    let has_route = !n.calls.is_empty() || !n.serves.is_empty() || !n.recv_routes.is_empty();
    if has_route {
        emit_route(n, s);
    }
    writeln!(s,"static wl_packet_submit_t {p}_submit(void *user, const uint8_t *data, size_t length, wl_packet_token_t token) {{\n  (void)token;").unwrap();
    if tx {
        writeln!(s,"  {p}_node_t *self = user;\n  wl_err_t error = self->tx_write(self, data, length);\n  if (error == WL_OK) return WL_PACKET_SENT;\n  return error == WL_ERR_WOULD_BLOCK || error == WL_ERR_BUSY ? WL_PACKET_BUSY : WL_PACKET_FAILED;\n}}").unwrap();
    } else {
        s.push_str("  (void)user; (void)data; (void)length;\n  return WL_PACKET_FAILED;\n}\n");
    }
    let tx_required = if tx { " || io->submit == NULL" } else { "" };
    writeln!(s,"wl_err_t {p}_init_io({p}_node_t *self, uint32_t instance, const wl_packet_io_t *io{extra}) {{\n  if (self == NULL || io == NULL{tx_required}) return WL_ERR_INVALID_ARG;\n  const wl_packet_io_t binding = *io;\n  const int external_rx = binding.rx_storage != NULL || binding.rx_storage_size != 0U || binding.rx_unit_size != 0U;\n  if (external_rx && (binding.rx_storage == NULL || binding.rx_unit_size < {upper}_RX_WIRE_MAX + 1U ||\n      binding.rx_unit_size > binding.rx_storage_size / WL_PACKET_RX_SLOTS)) return WL_ERR_INVALID_ARG;").unwrap();
    if dynamic {
        writeln!(s,"  if (handlers == NULL) return WL_ERR_INVALID_ARG;\n  const {p}_handlers_t saved = *handlers;").unwrap();
    }
    s.push_str("  wl_err_t error;\n  if (instance == 0U) {\n    error = wl_packet_instance_next(&instance);\n    if (error != WL_OK) return error;\n  }\n  memset(self, 0, sizeof(*self));\n");
    if dynamic {
        s.push_str("  self->handlers = saved; self->user = user;\n");
    }
    let physical = if tx {
        "binding.submit".to_owned()
    } else {
        format!("binding.submit != NULL ? binding.submit : {p}_submit")
    };
    let (submit, notify, context) = if stream {
        (
            format!("wl_packet_stream_{connector}_submit"),
            "binding.notify != NULL ? wl_packet_stream_notify : NULL",
            "&self->stream",
        )
    } else {
        (physical.clone(), "binding.notify", "binding.user")
    };
    writeln!(s,"  wl_packet_config_t packet = {{.instance = instance,\n      .rx_storage = external_rx ? binding.rx_storage : &self->rx[0][0],\n      .rx_unit_size = external_rx ? binding.rx_unit_size : sizeof(self->rx[0]),\n      .rx_storage_size = external_rx ? binding.rx_storage_size : sizeof(self->rx),\n      .submit = {submit}, .notify = {notify}, .user = {context}}};\n  error = wl_packet_init(&self->port, &packet);\n  if (error != WL_OK) return error;").unwrap();
    if stream {
        writeln!(s,"  wl_packet_stream_config_t stream = {{.port = &self->port,\n      .tx_storage = {{self->tx_wire, sizeof(self->tx_wire)}}, .physical_submit = {physical},\n      .notify = binding.notify, .user = binding.user, .maximum_frame_size = {upper}_TX_FRAME_MAX,\n      .maximum_rx_frame_size = {upper}_RX_FRAME_MAX}};\n  error = wl_packet_stream_{connector}_init(&self->stream, &stream);\n  if (error != WL_OK) {{ wl_packet_stop(&self->port); return error; }}").unwrap();
    }
    writeln!(s,"  wl_endpoint_config_t config = {{.port = &self->port, .route = {}, .on_error = {p}_error, .user = self,\n      .storage = {{",if has_route{format!("{p}_route")}else{"NULL".into()}).unwrap();
    for (field, stride, cap, array) in [
        ("requests", "request_stride", c.request, true),
        ("replies", "reply_stride", c.reply, true),
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
    s.push_str("      }};\n  error = wl_endpoint_init(&self->core, &config);\n  if (error != WL_OK) wl_packet_stop(&self->port);\n  return error;\n}\n");
    if tx {
        writeln!(s,"static wl_err_t {p}_init_sync({p}_node_t *self, uint32_t instance{tx_arg}{extra}) {{\n  if (tx_write == NULL) return WL_ERR_INVALID_ARG;\n  const wl_packet_io_t io = {{.submit = {p}_submit, .user = self}};\n  wl_err_t error = {p}_init_io(self, instance, &io{args});\n  if (error == WL_OK) self->tx_write = tx_write;\n  return error;\n}}\nwl_err_t {p}_init({p}_node_t *self{tx_arg}{extra}) {{\n  return {p}_init_sync(self, 0U, tx_write{args});\n}}\nwl_err_t {p}_init_with_instance({p}_node_t *self, uint32_t instance{tx_arg}{extra}) {{\n  if (instance == 0U) return WL_ERR_INVALID_ARG;\n  return {p}_init_sync(self, instance, tx_write{args});\n}}").unwrap();
    } else {
        writeln!(s,"wl_err_t {p}_init({p}_node_t *self{extra}) {{\n  return {p}_init_io(self, 0U, &(wl_packet_io_t){{0}}{args});\n}}\nwl_err_t {p}_init_with_instance({p}_node_t *self, uint32_t instance{extra}) {{\n  if (instance == 0U) return WL_ERR_INVALID_ARG;\n  return {p}_init_io(self, instance, &(wl_packet_io_t){{0}}{args});\n}}").unwrap();
    }
    if stream {
        writeln!(s,"wl_err_t {p}_rx_feed({p}_node_t *self, const uint8_t *data, size_t length, size_t *consumed) {{\n  if (self == NULL) {{ if (consumed != NULL) *consumed = 0U; return WL_ERR_INVALID_ARG; }}\n  return wl_packet_stream_{connector}_feed(&self->stream, data, length, consumed);\n}}").unwrap();
    } else {
        writeln!(s,"wl_err_t {p}_rx_packet({p}_node_t *self, const uint8_t *data, size_t length) {{\n  if (self == NULL) return WL_ERR_INVALID_ARG;\n  if (length > {upper}_RX_WIRE_MAX) return WL_ERR_BUF_TOO_SMALL;\n  return wl_packet_rx_push(&self->port, data, length);\n}}").unwrap();
    }
    writeln!(s,"wl_err_t {p}_step({p}_node_t *self, uint32_t now) {{\n  return self == NULL ? WL_ERR_INVALID_ARG : wl_endpoint_step(&self->core, now, WL_ENDPOINT_BUDGET);\n}}\nwl_err_t {p}_close({p}_node_t *self) {{\n  if (self == NULL) return WL_ERR_INVALID_ARG;\n  wl_err_t error = wl_endpoint_close(&self->core);").unwrap();
    if stream {
        s.push_str("  if (error == WL_OK) error = wl_packet_stream_abort_rx(&self->stream);\n");
    }
    writeln!(s,"  return error;\n}}\nint {p}_quiescent(const {p}_node_t *self) {{\n  return self != NULL && wl_endpoint_quiescent(&self->core);\n}}").unwrap();
    if !n.calls.is_empty() {
        writeln!(s,"wl_err_t {p}_cancel({p}_node_t *self, wl_call_handle_t handle) {{\n  return self == NULL ? WL_ERR_INVALID_ARG : wl_endpoint_cancel(&self->core, handle);\n}}").unwrap();
    }
}
fn emit_calls(n: &Node, h: &mut String, s: &mut String) {
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
        for ex in [false, true] {
            let suffix = if ex { "_ex" } else { "" };
            let ctx = if ex {
                "call->user"
            } else {
                &format!("({p}_node_t *)core")
            };
            writeln!(s,"static void {p}_complete_{rpc}{suffix}(wl_endpoint_t *core, const wl_endpoint_call_t *call, wl_rpc_result_t result, const uint8_t *body, size_t length) {{\n  (void)core;\n  {resp}_t response;\n  const {resp}_t *view = NULL;\n  if (result.outcome == WL_RPC_SUCCESS) {{\n    if ({resp}_decode(body, length, &response) == WL_CODEC_OK) view = &response;\n    else result = (wl_rpc_result_t){{WL_RPC_LOCAL_ERROR, WL_ERR_CORRUPT_PAYLOAD}};\n  }}\n  (({p}_{rpc}{suffix}_cb_t)call->callback)({ctx}, result, view);\n}}").unwrap();
        }
        writeln!(s,"static wl_err_t {p}_call_{rpc}_start({p}_node_t *self, const {req}_t *request, uint32_t now, uint32_t timeout, uint8_t has_now, wl_endpoint_complete_fn complete, void (*callback)(void), void *user, wl_call_handle_t *out) {{\n  if (out != NULL) *out = (wl_call_handle_t){{0}};\n  if (self == NULL || request == NULL || callback == NULL || timeout > INT32_MAX) return WL_ERR_INVALID_ARG;\n  if (timeout == 0U) timeout = {}U;\n  uint8_t slot; uint8_t *payload; size_t capacity;\n  wl_err_t error = wl_endpoint_call_claim(&self->core, &slot, &payload, &capacity);\n  if (error != WL_OK) return error;\n  size_t length = 0U;\n  if ({req}_encode(request, payload, capacity, &length) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;\n  return wl_endpoint_call_commit(&self->core, slot, {}U, {}U, length, now, timeout, has_now, complete, callback, user, out);\n}}",n.timeout_ms,r.request_id,r.response_id).unwrap();
        let callback = r.handler.as_deref().unwrap_or("callback");
        writeln!(s,"wl_err_t {p}_call_{rpc}({p}_node_t *self, const {req}_t *request{cb}) {{\n  return {p}_call_{rpc}_start(self, request, 0U, 0U, 0U, {p}_complete_{rpc}, (void (*)(void)){callback}, self, NULL);\n}}\nwl_err_t {p}_call_{rpc}_ex({p}_node_t *self, const {req}_t *request, uint32_t now, uint32_t timeout_ms, {p}_{rpc}_ex_cb_t callback, void *user, wl_call_handle_t *out) {{\n  return {p}_call_{rpc}_start(self, request, now, timeout_ms, 1U, {p}_complete_{rpc}_ex, (void (*)(void))callback, user, out);\n}}").unwrap();
    }
}
fn emit_route(n: &Node, s: &mut String) {
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
        writeln!(s,"    wl_rpc_metadata_t metadata;\n    wl_err_t error = wl_wire_rpc_read(frame->kind, frame->payload, frame->length, &metadata);\n    if (error != WL_OK) return error;\n    uint8_t slot; uint8_t *payload; size_t capacity;\n    error = wl_endpoint_reply_claim(core, {}U, &metadata, &slot, &payload, &capacity);\n    if (error != WL_OK) return error;\n    {req}_t request;\n    if ({req}_decode(metadata.body, metadata.length, &request) != WL_CODEC_OK) {{\n      wl_endpoint_reply_abort(core, slot); return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    {resp}_clear(&self->reply.{rpc});",r.response_id).unwrap();
        let handler = if dynamic {
            format!("self->handlers.on_{rpc}")
        } else {
            r.handler.clone().unwrap_or_else(|| format!("{p}_on_{rpc}"))
        };
        let user = if dynamic { "self->user" } else { "self" };
        writeln!(s,"    int32_t status = {handler}({user}, &request, &self->reply.{rpc});\n    if (status != 0) return wl_endpoint_reply_error(core, slot, status);\n    size_t length = 0U;\n    if ({resp}_encode(&self->reply.{rpc}, payload, capacity, &length) != WL_CODEC_OK) {{\n      wl_endpoint_reply_abort(core, slot); return WL_ERR_CORRUPT_PAYLOAD;\n    }}\n    return wl_endpoint_reply_commit(core, slot, length);\n  }}").unwrap();
    }
    for r in &n.recv_routes {
        let m = type_name(&r.message_name);
        let handler = if dynamic {
            format!("self->handlers.on_{m}")
        } else {
            r.handler.clone().unwrap_or_else(|| format!("{p}_on_{m}"))
        };
        let user = if dynamic { "self->user" } else { "self" };
        writeln!(s,"  case {}U: {{\n    if (frame->kind != WL_WIRE_MSG) return WL_ERR_CORRUPT_PAYLOAD;\n    {m}_t message;\n    if ({m}_decode(frame->payload, frame->length, &message) != WL_CODEC_OK) return WL_ERR_CORRUPT_PAYLOAD;",r.message_id).unwrap();
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
