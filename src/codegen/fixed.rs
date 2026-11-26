// SPDX-License-Identifier: Apache-2.0
use std::fmt::Write;

use super::{
    MessageDirections, Symbols, c_type,
    descriptor::field_descriptor_data,
    plan::{FieldKey, MessagePlan},
    type_name,
};
use crate::{
    ast::Cardinality,
    codec_plan::{CodecProfile, ValueEncoding, field_encoding},
    semantic::{FieldDefault, FieldSymbol, ResolvedType},
};

pub(super) fn supports(plan: &MessagePlan<'_>) -> bool {
    if plan.message.fields.len() > 8 {
        return false;
    }
    if plan.profile == CodecProfile::SchemaV1 && plan.message.fields.is_empty() {
        return false;
    }
    plan.message.fields.iter().all(|field| {
        matches!(
            field.cardinality,
            Cardinality::Optional | Cardinality::Required
        ) && !matches!(
            field.ty,
            ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. }
        )
    })
}

pub(super) fn same_shape(a: &MessagePlan<'_>, b: &MessagePlan<'_>) -> bool {
    a.profile == b.profile
        && a.message.fields.len() == b.message.fields.len()
        && a.message
            .fields
            .iter()
            .zip(&b.message.fields)
            .all(|(left, right)| {
                left.number == right.number
                    && left.cardinality == right.cardinality
                    && left.ty == right.ty
                    && left.default == right.default
            })
}

pub(super) fn shape_name(index: usize) -> String {
    format!("wlc_scalar_shape_{index}")
}

pub(super) fn max_field_value_bytes(plan: &MessagePlan<'_>, field: &FieldSymbol) -> usize {
    match field_encoding(field, plan.profile) {
        ValueEncoding::Bool => 1,
        ValueEncoding::UnsignedVarint(bits) | ValueEncoding::ZigzagVarint(bits) => match bits {
            8 => 2,
            16 => 3,
            32 => 5,
            _ => 10,
        },
        ValueEncoding::EnumZigzag => 5,
        ValueEncoding::LittleEndian(w) | ValueEncoding::IeeeLittleEndian(w) => usize::from(w),
        ValueEncoding::Bytes | ValueEncoding::Utf8 | ValueEncoding::Message(_) => 0,
    }
}

pub(super) fn shape_struct_def(plan: &MessagePlan<'_>, name: &str) -> String {
    let mut out = format!("typedef struct {name}_s {{\n");
    if plan.message.fields.is_empty() {
        writeln!(out, "  uint8_t _empty;").unwrap();
    } else {
        for (i, field) in plan.message.fields.iter().enumerate() {
            if field.cardinality == Cardinality::Optional {
                writeln!(out, "  bool h{i};").unwrap();
            }
            writeln!(out, "  {} v{i};", c_type(&field.ty)).unwrap();
        }
    }
    writeln!(out, "}} {name}_t;\n").unwrap();
    out
}

pub(super) fn shape_source(
    plan: &MessagePlan<'_>,
    name: &str,
    directions: MessageDirections,
    symbols: &Symbols,
) -> String {
    if plan.profile == CodecProfile::PositionalV1 {
        return super::positional_fixed::source(plan, name, directions, symbols);
    }
    schema_v1_shape_source(plan, name, directions)
}

pub(super) fn wrappers(
    plan: &MessagePlan<'_>,
    shape: &str,
    directions: MessageDirections,
    symbols: &Symbols,
) -> String {
    let name = symbols.message_name(plan.message);
    let ty = type_name(&plan.message.name);
    let storage = symbols.storage;
    let mut out = String::new();
    writeln!(
        out,
        "{storage}void {name}_clear({ty}_t *value) {{\n  _Static_assert(sizeof({ty}_t) == sizeof({shape}_t), \"shape layout mismatch\");\n  {shape}_clear(value);\n}}"
    )
    .unwrap();
    if directions.encode {
        writeln!(
            out,
            "{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ return {shape}_encoded_size(value); }}"
        )
        .unwrap();
        writeln!(
            out,
            "{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length) {{ return {shape}_encode(value, out, out_capacity, out_length); }}"
        )
        .unwrap();
    }
    if directions.decode {
        writeln!(
            out,
            "{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {ty}_t *out) {{\n  if (out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  {ty}_t temp;\n  wl_codec_status_t status = {shape}_decode(input, input_length, &temp);\n  if (status == WL_CODEC_OK) memcpy(out, &temp, sizeof(temp));\n  return status;\n}}"
        )
        .unwrap();
    }
    out.push('\n');
    out
}

fn emit_shape_clear(out: &mut String, plan: &MessagePlan<'_>, name: &str) {
    writeln!(
        out,
        "static void {name}_clear(void *value) {{\n  if (value == NULL) return;\n  {name}_t temp = {{0}};"
    )
    .unwrap();
    for (i, field) in plan.message.fields.iter().enumerate() {
        let Some(default) = &field.default else {
            continue;
        };
        let (_, signed, unsigned, _, _) = field_descriptor_data(field);
        match default {
            FieldDefault::Bool(b) if *b => writeln!(out, "  temp.v{i} = true;").unwrap(),
            FieldDefault::Int8(_)
            | FieldDefault::Int16(_)
            | FieldDefault::Int32(_)
            | FieldDefault::Int64(_)
            | FieldDefault::Enum(_)
                if signed != 0 =>
            {
                writeln!(out, "  temp.v{i} = ({})({signed}LL);", c_type(&field.ty)).unwrap();
            }
            _ if unsigned != 0 => {
                writeln!(
                    out,
                    "  temp.v{i} = ({})UINT64_C({unsigned});",
                    c_type(&field.ty)
                )
                .unwrap();
            }
            _ => {}
        }
    }
    writeln!(out, "  memcpy(value, &temp, sizeof(temp));\n}}\n").unwrap();
}

pub(super) fn emit_shape_default_clear(out: &mut String, plan: &MessagePlan<'_>, name: &str) {
    emit_shape_clear(out, plan, name);
}

fn expected_key(key: &FieldKey) -> String {
    if key.length == 1 {
        format!("cursor != end && *cursor == 0x{:02x}U", key.bytes[0])
    } else {
        let mut bytes = String::new();
        for byte in &key.bytes[..key.length] {
            write!(bytes, "\\x{byte:02x}").unwrap();
        }
        format!(
            "cursor != end && (size_t)(end - cursor) >= {0}U && memcmp(cursor, \"{bytes}\", {0}U) == 0",
            key.length
        )
    }
}

// Try the encoder's field order without a key dispatch or seen mask. A miss
// leaves out untouched, so the regular decoder owns all diagnostics.
fn emit_ordered_decode(out: &mut String, plan: &MessagePlan<'_>, name: &str) {
    writeln!(out, "static wl_codec_status_t {name}_ordered(const uint8_t *cursor, const uint8_t *end, void *out) {{\n  {name}_t temp;\n  {name}_clear(&temp);").unwrap();
    for (i, field) in plan.message.fields.iter().enumerate() {
        let key = &plan.keys[i];
        let matches = expected_key(key);
        writeln!(out, "  if ({matches}) {{\n    cursor += {}U;", key.length).unwrap();
        emit_scalar_read(out, field, &format!("temp.v{i}"), "    ", false);
        if field.cardinality == Cardinality::Optional {
            writeln!(out, "    temp.h{i} = true;\n  }}").unwrap();
        } else {
            writeln!(out, "  }} else return WL_CODEC_ERR_MALFORMED;").unwrap();
        }
    }
    writeln!(out, "  if (cursor != end) return WL_CODEC_ERR_MALFORMED;\n  memcpy(out, &temp, sizeof(temp));\n  return WL_CODEC_OK;\n}}\n").unwrap();
}

fn schema_v1_shape_source(
    plan: &MessagePlan<'_>,
    name: &str,
    directions: MessageDirections,
) -> String {
    let mut out = shape_struct_def(plan, name);
    emit_shape_clear(&mut out, plan, name);

    let max_size: usize = plan
        .message
        .fields
        .iter()
        .enumerate()
        .map(|(i, field)| plan.keys[i].length + max_field_value_bytes(plan, field))
        .sum();
    let all_required_fixed = plan.message.fields.iter().all(|field| {
        field.cardinality == Cardinality::Required
            && matches!(
                field.ty,
                ResolvedType::Fixed32
                    | ResolvedType::Fixed64
                    | ResolvedType::Float32
                    | ResolvedType::Float64
            )
    });

    if directions.encode {
        if !all_required_fixed {
            writeln!(
                out,
                "static size_t {name}_measure_fields(const {name}_t *v) {{\n  (void)v;\n  size_t size = 0U;"
            )
            .unwrap();
            for (i, field) in plan.message.fields.iter().enumerate() {
                let klen = plan.keys[i].length;
                if field.cardinality != Cardinality::Required {
                    writeln!(out, "  if (v->h{i}) {{").unwrap();
                }
                match field.ty {
                    ResolvedType::Bool => writeln!(out, "    size += {}U;", klen + 1).unwrap(),
                    ResolvedType::Uint8 | ResolvedType::Uint16 | ResolvedType::Uint32 => {
                        writeln!(
                            out,
                            "    size += {klen}U + wl_codec_vsize((uint64_t)v->v{i});"
                        )
                        .unwrap();
                    }
                    ResolvedType::Uint64 => {
                        writeln!(out, "    size += {klen}U + wl_codec_vsize(v->v{i});").unwrap();
                    }
                    ResolvedType::Int8
                    | ResolvedType::Int16
                    | ResolvedType::Int32
                    | ResolvedType::Enum { .. } => {
                        writeln!(
                            out,
                            "    size += {klen}U + wl_codec_vsize((uint64_t)wl_codec_zz32(v->v{i}));"
                        )
                        .unwrap();
                    }
                    ResolvedType::Int64 => {
                        writeln!(
                            out,
                            "    size += {klen}U + wl_codec_vsize(wl_codec_zz64(v->v{i}));"
                        )
                        .unwrap();
                    }
                    ResolvedType::Fixed32 | ResolvedType::Float32 => {
                        writeln!(out, "    size += {}U;", klen + 4).unwrap();
                    }
                    ResolvedType::Fixed64 | ResolvedType::Float64 => {
                        writeln!(out, "    size += {}U;", klen + 8).unwrap();
                    }
                    ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => {
                        unreachable!()
                    }
                }
                if field.cardinality != Cardinality::Required {
                    writeln!(out, "  }}").unwrap();
                }
            }
            writeln!(out, "  return size;\n}}\n").unwrap();
        }

        writeln!(
            out,
            "static size_t {name}_encoded_size(const void *value) {{\n  if (value == NULL) return SIZE_MAX;\n  {name}_t v;\n  memcpy(&v, value, sizeof(v));"
        )
        .unwrap();
        if all_required_fixed {
            writeln!(out, "  return {max_size}U;\n}}\n").unwrap();
        } else {
            writeln!(out, "  return {name}_measure_fields(&v);\n}}\n").unwrap();
        }

        writeln!(
            out,
            "static wl_codec_status_t {name}_encode(const void *value, uint8_t *out, size_t capacity, size_t *length) {{\n  if (value == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  {name}_t v;\n  memcpy(&v, value, sizeof(v));"
        )
        .unwrap();
        if all_required_fixed {
            if max_size > 0 {
                writeln!(
                    out,
                    "  if (length == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  if (capacity < {max_size}U) return WL_CODEC_ERR_CAPACITY;"
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "  if (length == NULL) return WL_CODEC_ERR_INVALID_VALUE;"
                )
                .unwrap();
            }
        } else {
            writeln!(
                out,
                "  if (length == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  if (out == NULL || capacity < {max_size}U) {{\n    const size_t need = {name}_measure_fields(&v);\n    if (need != 0U && out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n    if (capacity < need) return WL_CODEC_ERR_CAPACITY;\n    if (need == 0U) {{ *length = 0U; return WL_CODEC_OK; }}\n  }}"
            )
            .unwrap();
        }
        writeln!(out, "  uint8_t *cursor = out;").unwrap();
        for (i, field) in plan.message.fields.iter().enumerate() {
            let key = &plan.keys[i];
            let indent = if field.cardinality != Cardinality::Required {
                writeln!(out, "  if (v.h{i}) {{").unwrap();
                "    "
            } else {
                "  "
            };
            for byte in &key.bytes[..key.length] {
                writeln!(out, "{indent}*cursor++ = 0x{byte:02x}U;").unwrap();
            }
            emit_scalar_write(&mut out, field, &format!("v.v{i}"), indent, false);
            if field.cardinality != Cardinality::Required {
                writeln!(out, "  }}").unwrap();
            }
        }
        writeln!(
            out,
            "  *length = (size_t)(cursor - out);\n  return WL_CODEC_OK;\n}}\n"
        )
        .unwrap();
    }

    if directions.decode {
        let single = plan.keys.len() == 1;
        if !single {
            emit_ordered_decode(&mut out, plan, name);
        }
        writeln!(
            out,
            "static wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, void *out) {{\n  if (out == NULL || (length != 0U && input == NULL)) return WL_CODEC_ERR_INVALID_VALUE;\n  const uint8_t *cursor = input;\n  const uint8_t *end = length != 0U ? input + length : input;"
        )
        .unwrap();
        if !single {
            writeln!(out, "  if (cursor != end && {name}_ordered(cursor, end, out) == WL_CODEC_OK) return WL_CODEC_OK;").unwrap();
        }
        writeln!(
            out,
            "  {name}_t temp;\n  {name}_clear(&temp);\n  uint8_t seen = 0U;"
        )
        .unwrap();
        if single {
            // With one field the normal value reader is already the full
            // decoder. Skip key dispatch, without emitting a second reader.
            let key = &plan.keys[0];
            writeln!(
                out,
                "  if ({}) {{ cursor += {}U; goto first_value; }}",
                expected_key(key),
                key.length
            )
            .unwrap();
        }
        writeln!(
            out,
            "  while (cursor < end) {{\n    uint64_t wide_key = 0U;\n    wl_codec_status_t status = wl_codec_getv(&cursor, end, &wide_key);\n    if (status != WL_CODEC_OK) return status;\n    const uint64_t number = wide_key >> 3U;\n    const uint8_t wire = (uint8_t)(wide_key & 7U);\n    if (number == 0U || number > UINT16_MAX || (wire != 0U && wire != 1U && wire != 2U && wire != 5U))\n      return WL_CODEC_ERR_MALFORMED;\n    switch ((uint16_t)number) {{"
        )
        .unwrap();
        for (i, field) in plan.message.fields.iter().enumerate() {
            let wire = plan.keys[i].wire;
            let bit = 1_u8 << i;
            let value_label = if single { "\n      first_value: ;" } else { "" };
            writeln!(
                out,
                "      case {}U: {{\n        if (wire != {wire}U) return WL_CODEC_ERR_WIRE_TYPE;\n        if ((seen & {bit}U) != 0U) return WL_CODEC_ERR_DUPLICATE_FIELD;{value_label}",
                field.number
            )
            .unwrap();
            emit_scalar_read(&mut out, field, &format!("temp.v{i}"), "        ", false);
            if field.cardinality == Cardinality::Optional {
                writeln!(out, "        temp.h{i} = true;").unwrap();
            }
            writeln!(out, "        seen |= {bit}U;\n        break;\n      }}").unwrap();
        }
        writeln!(
            out,
            "      default: {{\n        status = wl_codec_skip(&cursor, end, wire);\n        if (status != WL_CODEC_OK) return status;\n        break;\n      }}\n    }}\n  }}"
        )
        .unwrap();
        let required_mask =
            plan.message
                .fields
                .iter()
                .enumerate()
                .fold(0_u8, |mask, (i, field)| {
                    if field.cardinality == Cardinality::Required {
                        mask | (1_u8 << i)
                    } else {
                        mask
                    }
                });
        if required_mask != 0 {
            writeln!(out, "  if ((seen & {required_mask}U) != {required_mask}U) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;").unwrap();
        }
        writeln!(
            out,
            "  memcpy(out, &temp, sizeof(temp));\n  return WL_CODEC_OK;\n}}\n"
        )
        .unwrap();
    }

    out
}

pub(super) fn emit_scalar_write(
    out: &mut String,
    field: &FieldSymbol,
    expr: &str,
    indent: &str,
    positional_fixed32_varint: bool,
) {
    match field.ty {
        ResolvedType::Bool => {
            writeln!(out, "{indent}*cursor++ = ({expr}) ? 1U : 0U;").unwrap();
        }
        ResolvedType::Uint8 | ResolvedType::Uint16 | ResolvedType::Uint32 => {
            writeln!(
                out,
                "{indent}cursor = wl_codec_putv(cursor, (uint64_t){expr});"
            )
            .unwrap();
        }
        ResolvedType::Uint64 => {
            writeln!(out, "{indent}cursor = wl_codec_putv(cursor, {expr});").unwrap();
        }
        ResolvedType::Int8
        | ResolvedType::Int16
        | ResolvedType::Int32
        | ResolvedType::Enum { .. } => {
            writeln!(
                out,
                "{indent}cursor = wl_codec_putv(cursor, (uint64_t)wl_codec_zz32({expr}));"
            )
            .unwrap();
        }
        ResolvedType::Int64 => {
            writeln!(
                out,
                "{indent}cursor = wl_codec_putv(cursor, wl_codec_zz64({expr}));"
            )
            .unwrap();
        }
        ResolvedType::Fixed32 if positional_fixed32_varint => {
            writeln!(
                out,
                "{indent}cursor = wl_codec_putv(cursor, (uint64_t){expr});"
            )
            .unwrap();
        }
        ResolvedType::Fixed32 | ResolvedType::Float32 => {
            writeln!(
                out,
                "{indent}{{\n{indent}  uint32_t bits = 0U;\n{indent}  memcpy(&bits, &{expr}, 4U);\n{indent}  wl_codec_put_le32(cursor, bits);\n{indent}  cursor += 4U;\n{indent}}}"
            )
            .unwrap();
        }
        ResolvedType::Fixed64 | ResolvedType::Float64 => {
            writeln!(
                out,
                "{indent}{{\n{indent}  uint64_t bits = 0U;\n{indent}  memcpy(&bits, &{expr}, 8U);\n{indent}  wl_codec_put_le64(cursor, bits);\n{indent}  cursor += 8U;\n{indent}}}"
            )
            .unwrap();
        }
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => unreachable!(),
    }
}

pub(super) fn emit_scalar_read(
    out: &mut String,
    field: &FieldSymbol,
    dst: &str,
    indent: &str,
    positional_fixed32_varint: bool,
) {
    match field.ty {
        ResolvedType::Bool => {
            let err = if positional_fixed32_varint {
                "WL_CODEC_ERR_MALFORMED"
            } else {
                "WL_CODEC_ERR_INVALID_VALUE"
            };
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}if (w > 1U) return {err};\n{indent}{dst} = (w != 0U);"
            )
            .unwrap();
        }
        ResolvedType::Uint8 | ResolvedType::Uint16 | ResolvedType::Uint32 => {
            let limit = match field.ty {
                ResolvedType::Uint8 => "UINT8_MAX",
                ResolvedType::Uint16 => "UINT16_MAX",
                _ => "UINT32_MAX",
            };
            let ty = c_type(&field.ty);
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}if (w > {limit}) return WL_CODEC_ERR_OVERFLOW;\n{indent}{dst} = ({ty})w;"
            )
            .unwrap();
        }
        ResolvedType::Fixed32 if positional_fixed32_varint => {
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}if (w > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW;\n{indent}{dst} = (uint32_t)w;"
            )
            .unwrap();
        }
        ResolvedType::Uint64 => {
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}{dst} = w;"
            )
            .unwrap();
        }
        ResolvedType::Int8
        | ResolvedType::Int16
        | ResolvedType::Int32
        | ResolvedType::Enum { .. } => {
            let narrow = match field.ty {
                ResolvedType::Int8 => {
                    "if (sv < INT8_MIN || sv > INT8_MAX) return WL_CODEC_ERR_OVERFLOW;\n"
                }
                ResolvedType::Int16 => {
                    "if (sv < INT16_MIN || sv > INT16_MAX) return WL_CODEC_ERR_OVERFLOW;\n"
                }
                _ => "",
            };
            let ty = c_type(&field.ty);
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}if (w > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW;\n{indent}const int32_t sv = wl_codec_unzz32((uint32_t)w);\n{indent}{narrow}{dst} = ({ty})sv;"
            )
            .unwrap();
        }
        ResolvedType::Int64 => {
            writeln!(
                out,
                "{indent}uint64_t w = 0U;\n{indent}wl_codec_status_t s = wl_codec_getv(&cursor, end, &w);\n{indent}if (s != WL_CODEC_OK) return s;\n{indent}{dst} = wl_codec_unzz64(w);"
            )
            .unwrap();
        }
        ResolvedType::Fixed32 | ResolvedType::Float32 => {
            writeln!(
                out,
                "{indent}if ((size_t)(end - cursor) < 4U) return WL_CODEC_ERR_MALFORMED;\n{indent}const uint32_t bits = wl_codec_get_le32(cursor);\n{indent}cursor += 4U;\n{indent}memcpy(&{dst}, &bits, 4U);"
            )
            .unwrap();
        }
        ResolvedType::Fixed64 | ResolvedType::Float64 => {
            writeln!(
                out,
                "{indent}if ((size_t)(end - cursor) < 8U) return WL_CODEC_ERR_MALFORMED;\n{indent}const uint64_t bits = wl_codec_get_le64(cursor);\n{indent}cursor += 8U;\n{indent}memcpy(&{dst}, &bits, 8U);"
            )
            .unwrap();
        }
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => unreachable!(),
    }
}
