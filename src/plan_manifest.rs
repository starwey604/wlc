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
    kernel_profile::KernelError,
    schema_identity,
};

pub const BINDING_PLAN_FORMAT: &str = "wirelink-binding-plan-v2";

pub fn generate_binding_plan_manifest(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
) -> Result<String, KernelError> {
    let kernel = crate::KernelProfile::from_bindings(profile.clone())?;
    let generated = crate::generate_kernel_c(schema, &kernel, "plan")?;
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
    for method in &profile.rpc_services {
        groups
            .entry(method.codec)
            .or_default()
            .extend([method.request_id, method.response_id]);
    }
    let deployment = &generated.files["plan_kernel_deployment.json"];
    let mut out = format!(
        "{{\n  \"format\": \"{BINDING_PLAN_FORMAT}\",\n  \"schema_identity\": \"0x{:016x}\",\n  \"binding_profile_identity\": \"0x{:016x}\",\n  \"deployment\": {},\n  \"codecs\": [",
        schema_identity(schema),
        binding_profile_identity(profile),
        deployment.trim()
    );
    let mut index = 0;
    for (codec, ids) in groups {
        let plan =
            CodecPlan::selected(schema, codec, ids).map_err(|e| KernelError(e.to_string()))?;
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
