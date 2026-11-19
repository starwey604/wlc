// SPDX-License-Identifier: Apache-2.0
use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write,
};

use super::{
    MessageDirections, Symbols, c_identifier, c_type,
    names::c_string,
    plan::{Lookup, MessagePlan},
    type_name,
};
use crate::{
    ast::Cardinality,
    codec_plan::{CodecPlan, CodecProfile},
    semantic::{FieldDefault, FieldSymbol, MessageSymbol, ResolvedType},
};

pub(crate) fn tagged_dependencies(
    codecs: &CodecPlan<'_>,
    directions: &HashMap<u16, MessageDirections>,
    uses_generic_desc: &dyn Fn(u16) -> bool,
) -> BTreeSet<u16> {
    let mut needed = BTreeSet::new();
    for plan in codecs.messages.values().rev() {
        let dir = directions
            .get(&plan.message.id)
            .copied()
            .unwrap_or_default();
        if (dir.encode || dir.decode) && uses_generic_desc(plan.message.id) {
            needed.insert(plan.message.id);
        }
        if needed.contains(&plan.message.id) {
            for field in &plan.message.fields {
                if let ResolvedType::Message { id, .. } = field.ty {
                    needed.insert(id);
                }
            }
        }
    }
    needed
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_descriptor(
    out: &mut String,
    plan: &MessagePlan<'_>,
    symbols: &Symbols,
    emit_fields: bool,
    emit_trampolines: bool,
    directions: MessageDirections,
    has_repeated: bool,
    is_positional: bool,
) {
    let owned_tagged;
    let plan = if !is_positional && plan.profile == CodecProfile::PositionalV1 {
        owned_tagged = MessagePlan::with_profile(plan.message, CodecProfile::SchemaV1);
        &owned_tagged
    } else {
        plan
    };
    let message = plan.message;
    let name = symbols.message_name(message);
    let sym = if is_positional {
        format!("{name}_pos")
    } else {
        name.clone()
    };
    let ty = type_name(&message.name);

    let mut trampolines = String::new();
    if emit_trampolines {
        let mut trampoline = |signature: String, body: String| {
            writeln!(out, "static {signature};").unwrap();
            writeln!(trampolines, "static {signature} {{ {body} }}").unwrap();
        };
        let clear = if emit_fields {
            format!("wl_codec_clear(&{sym}_desc, value);")
        } else {
            format!("{name}_clear(value);")
        };
        trampoline(format!("void {name}_wlc_clear_v(void *value)"), clear);
        if directions.encode {
            let (measure, encode) = if emit_fields {
                (
                    format!("return wl_codec_measure(&{sym}_desc, value, out_size);"),
                    format!("return wl_codec_encode(&{sym}_desc, value, out, capacity, length);"),
                )
            } else {
                (
                    format!(
                        "const size_t size = {name}_encoded_size(value); if (size == SIZE_MAX) return WL_CODEC_ERR_INVALID_VALUE; *out_size = size; return WL_CODEC_OK;"
                    ),
                    format!("return {name}_encode(value, out, capacity, length);"),
                )
            };
            trampoline(
                format!(
                    "wl_codec_status_t {name}_wlc_measure_v(const void *value, size_t *out_size)"
                ),
                measure,
            );
            trampoline(
                format!(
                    "wl_codec_status_t {name}_wlc_encode_v(const void *value, uint8_t *out, size_t capacity, size_t *length)"
                ),
                encode,
            );
        }
        if directions.decode {
            let decode = if emit_fields {
                format!(
                    "{ty}_t scratch; return wl_codec_decode(&{sym}_desc, input, length, out, &scratch);"
                )
            } else {
                format!("return {name}_decode(input, length, out);")
            };
            trampoline(
                format!(
                    "wl_codec_status_t {name}_wlc_decode_v(const uint8_t *input, size_t length, void *out)"
                ),
                decode,
            );
        }
    }

    let min_number = message.fields.first().map_or(0, |f| f.number);
    let max_number = message.fields.last().map_or(0, |f| f.number);
    let presence_bytes = if plan.profile == CodecProfile::PositionalV1 {
        let opt_count = message
            .fields
            .iter()
            .filter(|f| {
                matches!(
                    f.cardinality,
                    Cardinality::Optional | Cardinality::Packed(_)
                )
            })
            .count();
        opt_count.div_ceil(8)
    } else {
        0
    };

    let (fields_ptr, field_count, lookup_const, dense_ptr) = if emit_fields {
        for field in &message.fields {
            if let Some(FieldDefault::String(s)) = &field.default {
                let ident = c_identifier(&field.name);
                let escaped = c_string(s);
                writeln!(
                    out,
                    "static const char {sym}_{ident}_default_str[] = {escaped};"
                )
                .unwrap();
            }
        }
        if !message.fields.is_empty() {
            writeln!(out, "static const wl_codec_field_t {sym}_fields[] = {{").unwrap();
            for (index, field) in message.fields.iter().enumerate() {
                let entry = format_field_entry(plan, &sym, &ty, message, field, index);
                writeln!(out, "  {entry},").unwrap();
            }
            writeln!(out, "}};").unwrap();
        }
        let dense_expr = if plan.lookup == Lookup::Dense && !message.fields.is_empty() {
            let span = usize::from(max_number - min_number) + 1;
            let mut map = vec!["UINT16_MAX".to_owned(); span];
            for (index, field) in message.fields.iter().enumerate() {
                map[usize::from(field.number - min_number)] = format!("{index}U");
            }
            writeln!(
                out,
                "static const uint16_t {sym}_dense[{span}] = {{ {} }};",
                map.join(", ")
            )
            .unwrap();
            format!("{sym}_dense")
        } else {
            "NULL".to_owned()
        };
        let lookup_const = match plan.lookup {
            Lookup::Linear => "WL_CODEC_LOOKUP_LINEAR",
            Lookup::Dense => "WL_CODEC_LOOKUP_DENSE",
            Lookup::Binary => "WL_CODEC_LOOKUP_BINARY",
        };
        let fields_expr = if message.fields.is_empty() {
            "NULL".to_owned()
        } else {
            format!("{sym}_fields")
        };
        (fields_expr, message.fields.len(), lookup_const, dense_expr)
    } else {
        (
            "NULL".to_owned(),
            0,
            "WL_CODEC_LOOKUP_LINEAR",
            "NULL".to_owned(),
        )
    };

    let clear_fn = if emit_trampolines {
        format!("{name}_wlc_clear_v")
    } else {
        "NULL".to_owned()
    };
    let measure_fn = if emit_trampolines && directions.encode {
        format!("{name}_wlc_measure_v")
    } else {
        "NULL".to_owned()
    };
    let encode_fn = if emit_trampolines && directions.encode {
        format!("{name}_wlc_encode_v")
    } else {
        "NULL".to_owned()
    };
    let decode_fn = if emit_trampolines && directions.decode {
        format!("{name}_wlc_decode_v")
    } else {
        "NULL".to_owned()
    };

    let repeated_flag = if has_repeated { 1 } else { 0 };
    writeln!(
        out,
        "static const wl_codec_desc_t {sym}_desc = {{\n  .fields = {fields_ptr},\n  .field_count = {field_count}U,\n  .struct_size = sizeof({ty}_t),\n  .presence_bytes = {presence_bytes}U,\n  .min_number = {min_number}U,\n  .max_number = {max_number}U,\n  .lookup = {lookup_const},\n  .has_repeated = {repeated_flag}U,\n  .dense_map = {dense_ptr},\n  .clear = {clear_fn},\n  .measure = {measure_fn},\n  .encode = {encode_fn},\n  .decode = {decode_fn}\n}};\n"
    )
    .unwrap();
    if !trampolines.is_empty() {
        out.push_str(&trampolines);
        out.push('\n');
    }
}

fn format_field_entry(
    plan: &MessagePlan<'_>,
    msg_name: &str,
    ty: &str,
    _message: &MessageSymbol,
    field: &FieldSymbol,
    index: usize,
) -> String {
    let ident = c_identifier(&field.name);
    let key = &plan.keys[index];
    let (mut kind, signed_default, unsigned_default, _, child) = field_descriptor_data(field);
    let is_packed = matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    );
    if plan.profile == CodecProfile::PositionalV1 && !is_packed && field.ty == ResolvedType::Fixed32
    {
        kind = "WL_CODEC_KIND_UINT32";
    }
    let cardinality = match field.cardinality {
        Cardinality::Optional => "WL_CODEC_CARD_OPTIONAL",
        Cardinality::Required => "WL_CODEC_CARD_REQUIRED",
        Cardinality::Repeated => "WL_CODEC_CARD_REPEATED",
        Cardinality::Packed(_) => "WL_CODEC_CARD_PACKED",
        Cardinality::RequiredPacked(_) => "WL_CODEC_CARD_REQUIRED_PACKED",
    };
    let wire_type = if plan.profile == CodecProfile::PositionalV1 && !is_packed {
        0
    } else {
        key.wire
    };
    let (scalar_size, is_signed) = scalar_metrics(&field.ty);
    let presence_offset = if matches!(
        field.cardinality,
        Cardinality::Optional | Cardinality::Packed(_)
    ) {
        format!("(uint16_t)offsetof({ty}_t, has_{ident})")
    } else {
        "0U".to_owned()
    };
    let value_offset = format!("(uint16_t)offsetof({ty}_t, {ident})");
    let (count_offset, capacity_offset, element_stride, max_length) = match field.cardinality {
        Cardinality::Repeated => (
            format!("(uint16_t)offsetof({ty}_t, {ident}_count)"),
            format!("(uint16_t)offsetof({ty}_t, {ident}_capacity)"),
            format!("(uint32_t)sizeof(*((({ty}_t *)0)->{ident}))"),
            field.max_length.unwrap_or(0),
        ),
        Cardinality::Packed(count) | Cardinality::RequiredPacked(count) => {
            ("0U".to_owned(), "0U".to_owned(), "0U".to_owned(), count)
        }
        Cardinality::Optional | Cardinality::Required => {
            if matches!(field.ty, ResolvedType::Bytes | ResolvedType::String)
                && field.max_length.is_some()
            {
                (
                    format!(
                        "(uint16_t)(offsetof({ty}_t, {ident}.data) - offsetof({ty}_t, {ident}))"
                    ),
                    "0U".to_owned(),
                    "0U".to_owned(),
                    field.max_length.unwrap_or(0),
                )
            } else {
                ("0U".to_owned(), "0U".to_owned(), "0U".to_owned(), 0)
            }
        }
    };

    let (has_default, default_bits, string_default) = match &field.default {
        Some(FieldDefault::String(s)) => (
            if s.is_empty() { 0 } else { 1 },
            format!("UINT64_C({})", s.len()),
            format!("{msg_name}_{ident}_default_str"),
        ),
        Some(FieldDefault::Bool(b)) => (
            if *b { 1 } else { 0 },
            if *b { "UINT64_C(1)" } else { "UINT64_C(0)" }.to_owned(),
            "NULL".to_owned(),
        ),
        Some(
            FieldDefault::Int8(_)
            | FieldDefault::Int16(_)
            | FieldDefault::Int32(_)
            | FieldDefault::Int64(_)
            | FieldDefault::Enum(_),
        ) => (
            if signed_default != 0 { 1 } else { 0 },
            format!("UINT64_C({})", signed_default as u64),
            "NULL".to_owned(),
        ),
        Some(_) => (
            if unsigned_default != 0 { 1 } else { 0 },
            format!("UINT64_C({unsigned_default})"),
            "NULL".to_owned(),
        ),
        None => (0, "UINT64_C(0)".to_owned(), "NULL".to_owned()),
    };

    let key_bytes = format!(
        "{{ 0x{:02x}U, 0x{:02x}U, 0x{:02x}U }}",
        key.bytes[0], key.bytes[1], key.bytes[2]
    );

    format!(
        "{{ .number = {}U, .kind = {kind}, .cardinality = {cardinality}, .wire_type = {wire_type}U, .scalar_size = {scalar_size}U, .is_signed = {is_signed}U, .key_len = {}U, .key = {key_bytes}, .has_default = {has_default}U, .presence_offset = {presence_offset}, .offset = {value_offset}, .count_offset = {count_offset}, .capacity_offset = {capacity_offset}, .max_length = {max_length}U, .element_stride = {element_stride}, .default_bits = {default_bits}, .string_default = {string_default}, .child = {child} }}",
        field.number, key.length
    )
}

fn scalar_metrics(ty: &ResolvedType) -> (u8, u8) {
    match ty {
        ResolvedType::Bool => (1, 0),
        ResolvedType::Uint8 => (1, 0),
        ResolvedType::Uint16 => (2, 0),
        ResolvedType::Uint32 | ResolvedType::Fixed32 | ResolvedType::Float32 => (4, 0),
        ResolvedType::Uint64 | ResolvedType::Fixed64 | ResolvedType::Float64 => (8, 0),
        ResolvedType::Int8 => (1, 1),
        ResolvedType::Int16 => (2, 1),
        ResolvedType::Int32 | ResolvedType::Enum { .. } => (4, 1),
        ResolvedType::Int64 => (8, 1),
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => (0, 0),
    }
}

pub(crate) fn field_descriptor_data(
    field: &FieldSymbol,
) -> (&'static str, i64, u64, String, String) {
    let inline = !matches!(field.cardinality, Cardinality::Repeated) && field.max_length.is_some();
    let kind = match &field.ty {
        ResolvedType::Bool => "WL_CODEC_KIND_BOOL",
        ResolvedType::Uint8 | ResolvedType::Uint16 | ResolvedType::Uint32 => "WL_CODEC_KIND_UINT32",
        ResolvedType::Uint64 => "WL_CODEC_KIND_UINT64",
        ResolvedType::Int8 | ResolvedType::Int16 | ResolvedType::Int32 => "WL_CODEC_KIND_INT32",
        ResolvedType::Int64 => "WL_CODEC_KIND_INT64",
        ResolvedType::Fixed32 => "WL_CODEC_KIND_FIXED32",
        ResolvedType::Fixed64 => "WL_CODEC_KIND_FIXED64",
        ResolvedType::Float32 => "WL_CODEC_KIND_FLOAT32",
        ResolvedType::Float64 => "WL_CODEC_KIND_FLOAT64",
        ResolvedType::Enum { .. } => "WL_CODEC_KIND_ENUM",
        ResolvedType::Bytes => {
            if inline {
                "WL_CODEC_KIND_INLINE_BYTES"
            } else {
                "WL_CODEC_KIND_BYTES"
            }
        }
        ResolvedType::String => {
            if inline {
                "WL_CODEC_KIND_INLINE_STRING"
            } else {
                "WL_CODEC_KIND_STRING"
            }
        }
        ResolvedType::Message { .. } => "WL_CODEC_KIND_MESSAGE",
    };
    let (signed, unsigned, literal) = match &field.default {
        Some(FieldDefault::Bool(value)) => (0, u64::from(*value), "NULL".to_owned()),
        Some(FieldDefault::String(value)) => (0, value.len() as u64, c_string(value)),
        Some(FieldDefault::Int8(value)) => (i64::from(*value), 0, "NULL".to_owned()),
        Some(FieldDefault::Uint8(value)) => (0, u64::from(*value), "NULL".to_owned()),
        Some(FieldDefault::Int16(value)) => (i64::from(*value), 0, "NULL".to_owned()),
        Some(FieldDefault::Uint16(value)) => (0, u64::from(*value), "NULL".to_owned()),
        Some(FieldDefault::Int32(value)) => (i64::from(*value), 0, "NULL".to_owned()),
        Some(FieldDefault::Uint32(value)) => (0, u64::from(*value), "NULL".to_owned()),
        Some(FieldDefault::Int64(value)) => (*value, 0, "NULL".to_owned()),
        Some(FieldDefault::Uint64(value)) => (0, *value, "NULL".to_owned()),
        Some(FieldDefault::Fixed32(value)) => (0, u64::from(*value), "NULL".to_owned()),
        Some(FieldDefault::Fixed64(value)) => (0, *value, "NULL".to_owned()),
        Some(FieldDefault::Enum(value)) => (i64::from(*value), 0, "NULL".to_owned()),
        None => (0, 0, "NULL".to_owned()),
    };
    let child = match &field.ty {
        ResolvedType::Message { name, .. } => format!("&{}_desc", type_name(name)),
        _ => "NULL".to_owned(),
    };
    let _ = c_type(&field.ty);
    (kind, signed, unsigned, literal, child)
}
