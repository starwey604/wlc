// SPDX-License-Identifier: Apache-2.0
use std::fmt::Write;

use super::{
    MessageDirections, Symbols,
    fixed::{
        emit_scalar_read, emit_scalar_write, emit_shape_default_clear, max_field_value_bytes,
        shape_struct_def,
    },
    plan::MessagePlan,
};
use crate::{ast::Cardinality, semantic::ResolvedType};

pub(super) fn source(
    plan: &MessagePlan<'_>,
    name: &str,
    directions: MessageDirections,
    _symbols: &Symbols,
) -> String {
    let mut out = shape_struct_def(plan, name);
    emit_shape_default_clear(&mut out, plan, name);

    let optional: Vec<usize> = plan
        .message
        .fields
        .iter()
        .enumerate()
        .filter_map(|(i, f)| (f.cardinality == Cardinality::Optional).then_some(i))
        .collect();
    let presence_bytes = usize::from(!optional.is_empty());
    let max_size: usize = presence_bytes
        + plan
            .message
            .fields
            .iter()
            .map(|field| max_field_value_bytes(plan, field))
            .sum::<usize>();
    let all_required_fixed = plan.message.fields.iter().all(|field| {
        field.cardinality == Cardinality::Required
            && matches!(
                field.ty,
                ResolvedType::Fixed64 | ResolvedType::Float32 | ResolvedType::Float64
            )
    });

    if directions.encode {
        if !all_required_fixed {
            writeln!(
                out,
                "static size_t {name}_measure_fields(const {name}_t *v) {{\n  (void)v;\n  size_t size = {presence_bytes}U;"
            )
            .unwrap();
            for (i, field) in plan.message.fields.iter().enumerate() {
                if field.cardinality != Cardinality::Required {
                    writeln!(out, "  if (v->h{i}) {{").unwrap();
                }
                match field.ty {
                    ResolvedType::Bool => writeln!(out, "    size += 1U;").unwrap(),
                    ResolvedType::Uint8
                    | ResolvedType::Uint16
                    | ResolvedType::Uint32
                    | ResolvedType::Fixed32 => {
                        writeln!(out, "    size += wl_codec_vsize((uint64_t)v->v{i});").unwrap();
                    }
                    ResolvedType::Uint64 => {
                        writeln!(out, "    size += wl_codec_vsize(v->v{i});").unwrap();
                    }
                    ResolvedType::Int8
                    | ResolvedType::Int16
                    | ResolvedType::Int32
                    | ResolvedType::Enum { .. } => {
                        writeln!(
                            out,
                            "    size += wl_codec_vsize((uint64_t)wl_codec_zz32(v->v{i}));"
                        )
                        .unwrap();
                    }
                    ResolvedType::Int64 => {
                        writeln!(out, "    size += wl_codec_vsize(wl_codec_zz64(v->v{i}));")
                            .unwrap();
                    }
                    ResolvedType::Float32 => writeln!(out, "    size += 4U;").unwrap(),
                    ResolvedType::Fixed64 | ResolvedType::Float64 => {
                        writeln!(out, "    size += 8U;").unwrap();
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
        for (i, field) in plan.message.fields.iter().enumerate() {
            if field.cardinality == Cardinality::Required {
                writeln!(out, "  if (!v.h{i}) return SIZE_MAX;").unwrap();
            }
        }
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
        for (i, field) in plan.message.fields.iter().enumerate() {
            if field.cardinality == Cardinality::Required {
                writeln!(
                    out,
                    "  if (!v.h{i}) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;"
                )
                .unwrap();
            }
        }
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
                    "  (void)capacity;\n  (void)out;\n  if (length == NULL) return WL_CODEC_ERR_INVALID_VALUE;"
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
        if !optional.is_empty() {
            let bits = optional
                .iter()
                .enumerate()
                .map(|(bit, idx)| format!("(v.h{idx} ? {}U : 0U)", 1_u16 << bit))
                .collect::<Vec<_>>();
            writeln!(out, "  *cursor++ = (uint8_t)({});", bits.join(" | ")).unwrap();
        }
        for (i, field) in plan.message.fields.iter().enumerate() {
            let indent = if field.cardinality != Cardinality::Required {
                writeln!(out, "  if (v.h{i}) {{").unwrap();
                "    "
            } else {
                "  "
            };
            emit_scalar_write(&mut out, field, &format!("v.v{i}"), indent, true);
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
        writeln!(
            out,
            "static wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, void *out) {{\n  if (out == NULL || (length != 0U && input == NULL)) return WL_CODEC_ERR_INVALID_VALUE;\n  {name}_t temp;\n  {name}_clear(&temp);\n  const uint8_t *cursor = input;\n  const uint8_t *end = length != 0U ? input + length : input;"
        )
        .unwrap();
        if !optional.is_empty() {
            let opt_count = optional.len();
            writeln!(
                out,
                "  if (length < 1U) return WL_CODEC_ERR_MALFORMED;\n  const uint8_t bitmap = *cursor++;"
            )
            .unwrap();
            if opt_count < 8 {
                writeln!(
                    out,
                    "  if ((bitmap >> {opt_count}U) != 0U) return WL_CODEC_ERR_MALFORMED;"
                )
                .unwrap();
            }
        }
        let mut opt_bit = 0usize;
        for (i, field) in plan.message.fields.iter().enumerate() {
            let is_opt = field.cardinality == Cardinality::Optional;
            let indent = if is_opt {
                let mask = 1_u16 << opt_bit;
                opt_bit += 1;
                writeln!(out, "  if ((bitmap & {mask}U) != 0U) {{").unwrap();
                "    "
            } else {
                "  {"
            };
            if !is_opt {
                writeln!(out, "  {{").unwrap();
            }
            let inner = "    ";
            let _ = indent;
            emit_scalar_read(&mut out, field, &format!("temp.v{i}"), inner, true);
            writeln!(out, "{inner}temp.h{i} = true;\n  }}").unwrap();
        }
        writeln!(
            out,
            "  if (cursor != end) return WL_CODEC_ERR_MALFORMED;\n  memcpy(out, &temp, sizeof(temp));\n  return WL_CODEC_OK;\n}}\n"
        )
        .unwrap();
    }

    out
}
