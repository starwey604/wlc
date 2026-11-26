// SPDX-License-Identifier: Apache-2.0
//! Validated local node capabilities, separate from message wire identities.
use crate::{
    ast::{Cardinality, RouteKind, Schema},
    codec_plan::{CodecPlan, CodecProfile},
    codegen::is_c_keyword,
    semantic::{MessageSymbol, ResolvedType, SemanticModel, Symbol},
};
use miette::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Framing {
    #[default]
    Native,
    CobsCrc16,
    Length16,
}
impl Framing {
    pub fn parse(value: &str) -> Result<Self, NodeError> {
        match value {
            "native" => Ok(Self::Native),
            "cobs_crc16" => Ok(Self::CobsCrc16),
            "length16" => Ok(Self::Length16),
            _ => Err(error("framing must be native, cobs_crc16, or length16")),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::CobsCrc16 => "cobs_crc16",
            Self::Length16 => "length16",
        }
    }
    pub fn bound(self, canonical: u64) -> u64 {
        match self {
            Self::Native => canonical,
            Self::CobsCrc16 => canonical + 4 + (canonical + 2) / 254,
            Self::Length16 => canonical + 2,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Handlers {
    #[default]
    Static,
    Dynamic,
}
impl Handlers {
    pub fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageRoute {
    pub message_name: String,
    pub message_id: u16,
    pub codec: CodecProfile,
    pub handler: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcRoute {
    pub name: String,
    pub request_name: String,
    pub request_id: u16,
    pub response_name: String,
    pub response_id: u16,
    pub handler: Option<String>,
}
impl RpcRoute {
    pub fn metadata_size(&self) -> u64 {
        8
    }
    pub fn minimum_response_size(&self) -> u64 {
        12
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub name: String,
    pub framing: Framing,
    pub handlers: Handlers,
    pub timeout_ms: u32,
    pub rx_slots: u8,
    pub rx_unit_align: u16,
    pub send_routes: Vec<MessageRoute>,
    pub recv_routes: Vec<MessageRoute>,
    pub calls: Vec<RpcRoute>,
    pub serves: Vec<RpcRoute>,
}
impl Node {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            framing: Framing::Native,
            handlers: Handlers::Static,
            timeout_ms: 500,
            rx_slots: 2,
            rx_unit_align: 1,
            send_routes: Vec::new(),
            recv_routes: Vec::new(),
            calls: Vec::new(),
            serves: Vec::new(),
        }
    }
    pub fn has_rpc_client(&self) -> bool {
        !self.calls.is_empty()
    }
    pub fn has_rpc_server(&self) -> bool {
        !self.serves.is_empty()
    }
    pub fn apply_overrides(
        &mut self,
        framing: Option<Framing>,
        rx_slots: Option<u8>,
        rx_unit_align: Option<u16>,
    ) {
        self.framing = framing.unwrap_or(self.framing);
        self.rx_slots = rx_slots.unwrap_or(self.rx_slots);
        self.rx_unit_align = rx_unit_align.unwrap_or(self.rx_unit_align);
    }
}
#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("node: {0}")]
#[diagnostic(code(wlc::node))]
pub struct NodeError(pub String);
fn error(s: impl Into<String>) -> NodeError {
    NodeError(s.into())
}
pub fn parse_rx_slots(value: &str) -> Result<u8, NodeError> {
    match value.parse() {
        Ok(slots @ (2 | 4 | 8)) => Ok(slots),
        _ => Err(error("rx_slots must be 2, 4, or 8")),
    }
}
pub fn parse_rx_unit_align(value: &str) -> Result<u16, NodeError> {
    match value.parse() {
        Ok(align @ (1 | 64 | 512)) => Ok(align),
        _ => Err(error("rx_unit_align must be 1, 64, or 512")),
    }
}
fn codec(s: &str) -> Result<CodecProfile, NodeError> {
    match s {
        "schema_v1" => Ok(CodecProfile::SchemaV1),
        "positional_v1" => Ok(CodecProfile::PositionalV1),
        _ => Err(error(format!("unknown codec `{s}`"))),
    }
}
fn message<'a>(model: &'a SemanticModel, name: &str) -> Result<&'a MessageSymbol, NodeError> {
    model
        .declarations
        .iter()
        .find_map(|s| match s {
            Symbol::Message(m) if m.name == name => Some(m),
            _ => None,
        })
        .ok_or_else(|| error(format!("unknown message `{name}`")))
}
fn has_repeated(model: &SemanticModel, m: &MessageSymbol) -> bool {
    m.fields.iter().any(|f|matches!(f.cardinality,Cardinality::Repeated)||matches!(&f.ty,ResolvedType::Message{name,..} if has_repeated(model,message(model,name).expect("resolved child"))))
}
fn bounded(
    model: &SemanticModel,
    m: &MessageSymbol,
    c: CodecProfile,
    rpc: bool,
    receive: bool,
) -> Result<(), NodeError> {
    let plan = CodecPlan::selected(model, c, [m.id]).map_err(|e| error(e.to_string()))?;
    let maximum = plan.messages[&m.id].max_encoded_size.ok_or_else(|| {
        error(format!(
            "message `{}` requires a finite encoded bound",
            m.name
        ))
    })?;
    if maximum > if rpc { 2040 } else { 2048 } {
        return Err(error(format!(
            "message `{}` exceeds the 2048-byte payload / 2052-byte frame limit",
            m.name
        )));
    }
    if receive && has_repeated(model, m) {
        return Err(error(format!(
            "received message `{}` requires caller-owned repeated backing",
            m.name
        )));
    }
    Ok(())
}
pub fn analyze_nodes(ast: &Schema, model: &SemanticModel) -> Result<Vec<Node>, NodeError> {
    let mut signatures = BTreeMap::<String, (String, String)>::new();
    for rpc in &ast.rpcs {
        if signatures
            .insert(
                rpc.name.value.clone(),
                (rpc.request.value.clone(), rpc.response.value.clone()),
            )
            .is_some()
        {
            return Err(error(format!("duplicate RPC `{}`", rpc.name.value)));
        }
    }
    for node in &ast.nodes {
        for route in &node.routes {
            if let Some((req, resp)) = &route.signature {
                let signature = (req.value.clone(), resp.value.clone());
                if let Some(previous) =
                    signatures.insert(route.name.value.clone(), signature.clone())
                    && previous != signature
                {
                    return Err(error(format!(
                        "conflicting RPC signature `{}`",
                        route.name.value
                    )));
                }
            }
        }
    }
    for (name, (req, resp)) in &signatures {
        let req = message(model, req)?;
        let resp = message(model, resp)?;
        if req.id == resp.id {
            return Err(error(format!(
                "RPC `{name}` request and response must differ"
            )));
        }
    }
    let mut nodes = Vec::new();
    let mut names = BTreeSet::new();
    for declaration in &ast.nodes {
        let name = crate::codegen::type_name(&declaration.name.value);
        if !names.insert(name.clone()) {
            return Err(error(format!(
                "node names collide after C normalization: `{name}`"
            )));
        }
        let mut node = Node::new(name);
        let mut default_codec = CodecProfile::SchemaV1;
        let mut settings = BTreeSet::new();
        for (key, value) in &declaration.settings {
            if !settings.insert(&key.value) {
                return Err(error(format!("duplicate node setting `{}`", key.value)));
            }
            match (key.value.as_str(), value.value.as_str()) {
                ("framing", value) => node.framing = Framing::parse(value)?,
                ("handlers", "static") => node.handlers = Handlers::Static,
                ("handlers", "dynamic") => node.handlers = Handlers::Dynamic,
                ("codec", value) => default_codec = codec(value)?,
                ("rx_slots", value) => node.rx_slots = parse_rx_slots(value)?,
                ("rx_unit_align", value) => node.rx_unit_align = parse_rx_unit_align(value)?,
                ("timeout_ms", value) => {
                    node.timeout_ms = value
                        .parse::<u32>()
                        .map_err(|_| error("invalid timeout_ms"))?;
                    if node.timeout_ms == 0 || node.timeout_ms > i32::MAX as u32 {
                        return Err(error("timeout_ms must be in 1..=2147483647"));
                    }
                }
                _ => {
                    return Err(error(format!(
                        "invalid node setting {} = {}",
                        key.value, value.value
                    )));
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut ordinary = BTreeSet::new();
        let mut rpc_ids = BTreeMap::<u16, String>::new();
        for route in &declaration.routes {
            let handler = route.handler.as_ref().map(|h| h.value.clone());
            if let Some(h) = &handler
                && (is_c_keyword(h)
                    || !h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || !h.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    || h.starts_with('_')
                    || h.starts_with("wl_"))
            {
                return Err(error(format!("invalid or reserved C handler `{h}`")));
            }
            if node.handlers == Handlers::Dynamic
                && handler.is_some()
                && matches!(route.kind, RouteKind::Recv | RouteKind::Serve)
            {
                return Err(error(
                    "dynamic recv/serve handlers are supplied at init; omit = symbol",
                ));
            }
            let key = (format!("{:?}", route.kind), route.name.value.clone());
            if !seen.insert(key) {
                return Err(error(format!(
                    "duplicate {:?} route `{}`",
                    route.kind, route.name.value
                )));
            }
            match route.kind {
                RouteKind::Send | RouteKind::Recv => {
                    let m = message(model, &route.name.value)?;
                    let c = route
                        .codec
                        .as_ref()
                        .map(|v| codec(&v.value))
                        .transpose()?
                        .unwrap_or(default_codec);
                    bounded(model, m, c, false, route.kind == RouteKind::Recv)?;
                    ordinary.insert(m.id);
                    let r = MessageRoute {
                        message_name: m.name.clone(),
                        message_id: m.id,
                        codec: c,
                        handler,
                    };
                    if route.kind == RouteKind::Send {
                        node.send_routes.push(r)
                    } else {
                        node.recv_routes.push(r)
                    }
                }
                RouteKind::Call | RouteKind::Serve => {
                    let (request, response) =
                        signatures.get(&route.name.value).ok_or_else(|| {
                            error(format!(
                                "unknown RPC `{}`; declare its signature",
                                route.name.value
                            ))
                        })?;
                    let req = message(model, request)?;
                    let resp = message(model, response)?;
                    bounded(
                        model,
                        req,
                        CodecProfile::SchemaV1,
                        true,
                        route.kind == RouteKind::Serve,
                    )?;
                    bounded(
                        model,
                        resp,
                        CodecProfile::SchemaV1,
                        true,
                        route.kind == RouteKind::Call,
                    )?;
                    for id in [req.id, resp.id] {
                        if let Some(other) = rpc_ids.insert(id, route.name.value.clone())
                            && other != route.name.value
                        {
                            return Err(error(format!(
                                "RPC `{}` shares message ID {id} with `{other}`",
                                route.name.value
                            )));
                        }
                    }
                    let r = RpcRoute {
                        name: route.name.value.clone(),
                        request_name: req.name.clone(),
                        request_id: req.id,
                        response_name: resp.name.clone(),
                        response_id: resp.id,
                        handler,
                    };
                    if route.kind == RouteKind::Call {
                        node.calls.push(r)
                    } else {
                        node.serves.push(r)
                    }
                }
            }
        }
        if ordinary.iter().any(|id| rpc_ids.contains_key(id)) {
            return Err(error(
                "RPC messages cannot also be ordinary send/recv routes in the same node",
            ));
        }
        node.send_routes.sort_by_key(|r| r.message_id);
        node.recv_routes.sort_by_key(|r| r.message_id);
        node.calls.sort_by(|a, b| a.name.cmp(&b.name));
        node.serves.sort_by(|a, b| a.name.cmp(&b.name));
        nodes.push(node);
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(nodes)
}

pub fn select_nodes(nodes: &[Node], name: Option<&str>, all: bool) -> Result<Vec<Node>, NodeError> {
    if all && name.is_some() {
        return Err(error("--node and --all-nodes are mutually exclusive"));
    }
    if let Some(name) = name {
        return nodes
            .iter()
            .find(|n| n.name == crate::codegen::type_name(name))
            .map(|n| vec![n.clone()])
            .ok_or_else(|| error(format!("unknown node `{name}`")));
    }
    if nodes.len() > 1 && !all {
        return Err(error(
            "multiple nodes: specify --node <name> or --all-nodes",
        ));
    }
    Ok(nodes.to_vec())
}

pub fn sdk_node(
    ast: &Schema,
    model: &SemanticModel,
    nodes: &[Node],
    name: Option<&str>,
) -> Result<Node, NodeError> {
    if let Some(name) = name {
        return select_nodes(nodes, Some(name), false).map(|mut n| n.remove(0));
    }
    let clients: Vec<_> = nodes.iter().filter(|n| !n.calls.is_empty()).collect();
    match clients.as_slice() {
        [node] => Ok((*node).clone()),
        [] => {
            if ast.rpcs.is_empty() {
                return Err(error(
                    "SDK requires a call node or top-level RPC declarations",
                ));
            }
            let mut node = Node::new("client");
            let mut used = BTreeSet::new();
            for rpc in &ast.rpcs {
                let req = message(model, &rpc.request.value)?;
                let resp = message(model, &rpc.response.value)?;
                bounded(model, req, CodecProfile::SchemaV1, true, false)?;
                bounded(model, resp, CodecProfile::SchemaV1, true, true)?;
                if !used.insert(req.id) || !used.insert(resp.id) {
                    return Err(error(
                        "synthesized SDK client has conflicting RPC message IDs",
                    ));
                }
                node.calls.push(RpcRoute {
                    name: rpc.name.value.clone(),
                    request_name: req.name.clone(),
                    request_id: req.id,
                    response_name: resp.name.clone(),
                    response_id: resp.id,
                    handler: None,
                });
            }
            node.calls.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(node)
        }
        _ => Err(error("multiple client nodes: specify --node <name>")),
    }
}
