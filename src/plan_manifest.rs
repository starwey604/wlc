// SPDX-License-Identifier: Apache-2.0
//! Reviewable compiler plan; separate from a manifest claiming emitted artifacts.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use crate::{
    BindingProfileModel, SemanticModel,
    ast::Cardinality,
    binding_profile_identity,
    codec_plan::{CodecPlan, CodecProfile, ValueEncoding},
    manifest::push_json_string,
    profile_semantic::RpcGuarantee,
    rpc_plan::{RpcEndpointPlan, RpcPlanError},
    schema_identity,
};

pub const BINDING_PLAN_FORMAT: &str = "wirelink-binding-plan-v1";

pub fn generate_binding_plan_manifest(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
) -> Result<String, RpcPlanError> {
    let rpc = RpcEndpointPlan::new(schema, profile)?;
    let mut groups: BTreeMap<CodecProfile, BTreeSet<u16>> = BTreeMap::new();
    for (codec, id) in profile
        .send_routes
        .iter()
        .chain(&profile.direct_routes)
        .map(|r| (r.codec, r.message_id))
        .chain(
            profile
                .retained_routes
                .iter()
                .map(|r| (r.codec, r.message_id)),
        )
    {
        groups.entry(codec).or_default().insert(id);
    }
    for method in &rpc.methods {
        groups
            .entry(method.codec)
            .or_default()
            .extend([method.request_id, method.response_id]);
    }
    let execution_available = crate::runtime_codegen::validate_execution_policies(profile).is_ok();
    let mut out = format!(
        "{{\n  \"format\": \"{BINDING_PLAN_FORMAT}\",\n  \"schema_identity\": \"0x{:016x}\",\n  \"binding_profile_identity\": \"0x{:016x}\",\n  \"execution_generation_available\": {execution_available},\n  \"typed_result_contract\": {{\"major\": 0, \"minor\": {}}},\n  \"client_result_methods\": ",
        schema_identity(schema),
        binding_profile_identity(profile),
        crate::manifest::CODEGEN_CONTRACT_MINOR
    );
    strings(&mut out, &rpc.client_result_methods);
    out.push_str(",\n  \"pending_response_methods\": ");
    strings(&mut out, &rpc.pending_response_methods);
    out.push_str(",\n  \"replay_methods\": ");
    strings(&mut out, &rpc.replay_methods);
    writeln!(out, ",\n  \"request_snapshot_capacity\": {},\n  \"pending_response_capacity\": {},\n  \"replay_response_capacity\": {},", number(rpc.request_snapshot_capacity), number(rpc.pending_response_capacity), number(rpc.replay_response_capacity)).unwrap();
    out.push_str("  \"methods\": [");
    for (index, method) in rpc.methods.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str("\n    {\"name\": ");
        push_json_string(&mut out, &method.name);
        write!(out, ", \"request_id\": {}, \"response_id\": {}, \"guarantee\": \"{}\", \"codec\": \"{}\", \"request_capacity\": {}, \"response_capacity\": {}, \"owned_result_available\": {}}}", method.request_id, method.response_id, if method.guarantee == RpcGuarantee::Replay {"replay"} else {"correlate"}, method.codec.name(), number(method.request_capacity), number(method.response_capacity), method.response_has_owned_value).unwrap();
    }
    out.push_str("\n  ],\n  \"codecs\": [");
    let mut index = 0;
    for (codec, ids) in groups {
        let plan =
            CodecPlan::selected(schema, codec, ids).map_err(|e| RpcPlanError(e.to_string()))?;
        for (id, message) in plan.messages {
            if index != 0 {
                out.push(',');
            }
            index += 1;
            write!(out, "\n    {{\"message_id\": {id}, \"codec\": \"{}\", \"max_encoded_size\": {}, \"presence_bytes\": {}, \"fields\": [", codec.name(), number(message.max_encoded_size), message.presence_bytes).unwrap();
            for (n, field) in message.fields.iter().enumerate() {
                if n != 0 {
                    out.push_str(", ");
                }
                write!(
                    out,
                    "{{\"id\": {}, \"cardinality\": \"{}\", \"packed_count\": {}, \"max_length\": {}, \"encoding\": \"{}\", \"wire_type\": {}, \"presence_bit\": {}}}",
                    field.field.number,
                    cardinality(field.field.cardinality),
                    number(match field.field.cardinality { Cardinality::Packed(n) | Cardinality::RequiredPacked(n) => Some(u64::from(n)), _ => None }),
                    number(field.field.max_length.map(u64::from)),
                    encoding(field.encoding),
                    number(field.wire_type.map(u64::from)),
                    number(field.presence_bit.map(|n| n as u64))
                )
                .unwrap();
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
fn strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        push_json_string(out, value);
    }
    out.push(']');
}
fn encoding(value: ValueEncoding) -> String {
    match value {
        ValueEncoding::Bool => "bool".into(),
        ValueEncoding::UnsignedVarint(bits) => format!("uvar{bits}"),
        ValueEncoding::ZigzagVarint(bits) => format!("zigzag{bits}"),
        ValueEncoding::EnumZigzag => "enum-zigzag32".into(),
        ValueEncoding::BigEndian(bytes) => format!("be{}", bytes * 8),
        ValueEncoding::IeeeBigEndian(bytes) => format!("ieee-be{}", bytes * 8),
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
