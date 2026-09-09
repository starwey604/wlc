// SPDX-License-Identifier: Apache-2.0
use super::names::c_string;
use super::plan::{Lookup, MessagePlan};
use super::{c_identifier, c_type, type_name};
use crate::semantic::{FieldDefault, FieldSymbol, ResolvedType};

pub(super) fn emit_descriptor(output: &mut String, plan: &MessagePlan<'_>) {
    let message = plan.message;
    let name = type_name(&message.name);
    if message.fields.is_empty() {
        output.push_str(&format!(
            "static const wlc_desc_t {name}_desc = {{ NULL, 0U, WLC_LOOKUP_LINEAR }};\n\n"
        ));
        return;
    }
    output.push_str(&format!("static const wlc_field_t {name}_fields[] = {{\n"));
    for (field, key) in message.fields.iter().zip(&plan.keys) {
        let field_name = c_identifier(&field.name);
        let (kind, signed_default, unsigned_default, string_default, nested) =
            field_descriptor_data(field);
        let (cardinality, required, has, count, capacity, packed_count) = match field.cardinality {
            crate::ast::Cardinality::Optional => (
                "WLC_OPTIONAL",
                "0",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Required => (
                "WLC_OPTIONAL",
                "1",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Repeated => (
                "WLC_REPEATED",
                "0",
                "0".to_owned(),
                format!("offsetof({name}_t, {field_name}_count)"),
                format!("offsetof({name}_t, {field_name}_capacity)"),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Packed(element_count) => (
                "WLC_PACKED",
                "0",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                element_count.to_string(),
            ),
            crate::ast::Cardinality::RequiredPacked(element_count) => (
                "WLC_PACKED",
                "1",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                element_count.to_string(),
            ),
        };
        let max_length = field.max_length.unwrap_or(0);
        let wire = key.wire;
        let key_size = key.length;
        let [key0, key1, key2] = key.bytes;
        output.push_str(&format!("  {{ {}U, {cardinality}, {kind}, {required}, {wire}U, {key_size}U, offsetof({name}_t, {field_name}), {has}, {count}, {capacity}, sizeof({}), {packed_count}U, {max_length}U, {{ {key0}U, {key1}U, {key2}U }}, {signed_default}, {unsigned_default}ULL, {string_default}, {nested} }},\n", field.number, c_type(&field.ty)));
    }
    output.push_str("};\n");
    let lookup = match plan.lookup {
        Lookup::Linear => "WLC_LOOKUP_LINEAR",
        Lookup::Dense => "WLC_LOOKUP_DENSE",
        Lookup::Binary => "WLC_LOOKUP_BINARY",
    };
    output.push_str(&format!("static const wlc_desc_t {name}_desc = {{ {name}_fields, sizeof({name}_fields) / sizeof({name}_fields[0]), {lookup} }};\n\n"));
}

pub(crate) fn field_descriptor_data(
    field: &FieldSymbol,
) -> (&'static str, String, String, String, String) {
    let kind = match field.ty {
        ResolvedType::Bool => "WLC_BOOL",
        ResolvedType::Bytes => "WLC_BYTES",
        ResolvedType::String => "WLC_STRING",
        ResolvedType::Int8 => "WLC_I8",
        ResolvedType::Uint8 => "WLC_U8",
        ResolvedType::Int16 => "WLC_I16",
        ResolvedType::Uint16 => "WLC_U16",
        ResolvedType::Int32 => "WLC_I32",
        ResolvedType::Uint32 => "WLC_U32",
        ResolvedType::Int64 => "WLC_I64",
        ResolvedType::Uint64 => "WLC_U64",
        ResolvedType::Fixed32 => "WLC_F32",
        ResolvedType::Fixed64 => "WLC_F64",
        ResolvedType::Float32 => "WLC_FLOAT32",
        ResolvedType::Float64 => "WLC_FLOAT64",
        ResolvedType::Enum { .. } => "WLC_ENUM",
        ResolvedType::Message { .. } => "WLC_MESSAGE",
    };
    let nested = match &field.ty {
        ResolvedType::Message { name, .. } => format!("&{}_desc", type_name(name)),
        _ => "NULL".to_owned(),
    };
    let (signed, unsigned, string) = match &field.default {
        Some(FieldDefault::Bool(value)) => (
            "0".to_owned(),
            (*value as u8).to_string(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::String(value)) => {
            ("0".to_owned(), value.len().to_string(), c_string(value))
        }
        Some(FieldDefault::Int8(value)) => (value.to_string(), "0".to_owned(), "NULL".to_owned()),
        Some(FieldDefault::Uint8(value)) => ("0".to_owned(), value.to_string(), "NULL".to_owned()),
        Some(FieldDefault::Int16(value)) => (value.to_string(), "0".to_owned(), "NULL".to_owned()),
        Some(FieldDefault::Uint16(value)) => ("0".to_owned(), value.to_string(), "NULL".to_owned()),
        Some(FieldDefault::Int32(value)) | Some(FieldDefault::Enum(value)) => (
            format!("INT32_C({value})"),
            "0".to_owned(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::Int64(value)) => (
            if *value == i64::MIN {
                "INT64_MIN".to_owned()
            } else {
                format!("INT64_C({value})")
            },
            "0".to_owned(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::Uint32(value)) | Some(FieldDefault::Fixed32(value)) => {
            ("0".to_owned(), value.to_string(), "NULL".to_owned())
        }
        Some(FieldDefault::Uint64(value)) | Some(FieldDefault::Fixed64(value)) => {
            ("0".to_owned(), value.to_string(), "NULL".to_owned())
        }
        None => ("0".to_owned(), "0".to_owned(), "NULL".to_owned()),
    };
    (kind, signed, unsigned, string, nested)
}
