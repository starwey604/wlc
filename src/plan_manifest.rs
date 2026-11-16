// SPDX-License-Identifier: Apache-2.0
//! Reviewable node resource plan, separate from artifact provenance.
use crate::{
    SemanticModel,
    ast::Cardinality,
    codec_plan::{CodecPlan, CodecProfile, ValueEncoding},
    node::{Node, NodeError},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};
pub const NODE_PLAN_FORMAT: &str = "wirelink-node-plan-v1";
pub fn generate_node_plan_manifest(
    schema: &SemanticModel,
    nodes: &[Node],
) -> Result<String, NodeError> {
    let generated = crate::generate_node_c(schema, nodes, "plan")?;
    let manifest = &generated.files["plan_manifest.json"];
    let metadata = &manifest
        [manifest.find("\"nodes\":").expect("node metadata")..manifest.rfind('}').unwrap()];
    let mut out =
        format!("{{\n  \"format\": \"{NODE_PLAN_FORMAT}\",\n  {metadata},\n  \"codecs\": [");
    let mut groups = BTreeMap::<CodecProfile, BTreeSet<u16>>::new();
    for node in nodes {
        for r in node.send_routes.iter().chain(&node.recv_routes) {
            groups.entry(r.codec).or_default().insert(r.message_id);
        }
        for r in node.calls.iter().chain(&node.serves) {
            groups
                .entry(CodecProfile::SchemaV1)
                .or_default()
                .extend([r.request_id, r.response_id]);
        }
    }
    if nodes.is_empty() {
        groups.insert(
            CodecProfile::SchemaV1,
            schema
                .declarations
                .iter()
                .filter_map(|s| match s {
                    crate::semantic::Symbol::Message(m) => Some(m.id),
                    _ => None,
                })
                .collect(),
        );
    }
    let mut index = 0;
    for (codec, ids) in groups {
        let plan = CodecPlan::selected(schema, codec, ids).map_err(|e| NodeError(e.to_string()))?;
        for (id, message) in plan.messages {
            if index != 0 {
                out.push(',');
            }
            index += 1;
            write!(out,"\n    {{\"message_id\": {id}, \"codec\": \"{}\", \"max_encoded_size\": {}, \"presence_bytes\": {}, \"fields\": [",codec.name(),number(message.max_encoded_size),message.presence_bytes).unwrap();
            for (i, f) in message.fields.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                write!(out,"{{\"id\":{},\"cardinality\":\"{}\",\"encoding\":\"{}\",\"presence_bit\":{},\"wire_type\":{},\"max_length\":{},\"packed_count\":{}}}",f.field.number,cardinality(f.field.cardinality),encoding(f.encoding),number(f.presence_bit.map(|n|n as u64)),number(f.wire_type.map(u64::from)),number(f.field.max_length.map(u64::from)),number(match f.field.cardinality { Cardinality::Packed(n)|Cardinality::RequiredPacked(n)=>Some(u64::from(n)),_=>None })).unwrap();
            }
            out.push_str("]}");
        }
    }
    out.push_str("\n  ]\n}\n");
    Ok(out)
}
fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |n| n.to_string())
}
fn encoding(value: ValueEncoding) -> String {
    match value {
        ValueEncoding::Bool => "bool".into(),
        ValueEncoding::UnsignedVarint(bits) => format!("uvar{bits}"),
        ValueEncoding::ZigzagVarint(bits) => format!("zigzag{bits}"),
        ValueEncoding::EnumZigzag => "enum-zigzag32".into(),
        ValueEncoding::LittleEndian(bytes) => format!("le{}", bytes * 8),
        ValueEncoding::IeeeLittleEndian(bytes) => format!("ieee-le{}", bytes * 8),
        ValueEncoding::Bytes => "bytes".into(),
        ValueEncoding::Utf8 => "utf8".into(),
        ValueEncoding::Message(id) => format!("message-{id}"),
    }
}

fn cardinality(value: Cardinality) -> &'static str {
    match value {
        Cardinality::Optional => "optional",
        Cardinality::Required => "required",
        Cardinality::Repeated => "repeated",
        Cardinality::Packed(_) => "packed",
        Cardinality::RequiredPacked(_) => "required_packed",
    }
}
