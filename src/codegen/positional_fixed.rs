// SPDX-License-Identifier: Apache-2.0
//! The existing bounded scalar shape, with positional framing selected by plan.
use super::{
    Symbols,
    descriptor::field_descriptor_data,
    fixed::{access, emit_bits, width},
    plan::MessagePlan,
};
use crate::{ast::Cardinality, semantic::ResolvedType};
use std::fmt::Write;

pub(super) fn source(
    plan: &MessagePlan<'_>,
    name: &str,
    fingerprint: bool,
    symbols: &Symbols,
) -> String {
    let p = &symbols.helper;
    let optional = plan
        .message
        .fields
        .iter()
        .filter(|f| f.cardinality == Cardinality::Optional)
        .count();
    let presence = usize::from(optional != 0);
    let mut out = format!(
        "static inline wl_codec_status_t {name}_wlc_detail_measure(const void *value, size_t *size) {{\n  if (value == NULL || size == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n"
    );
    let required = plan
        .message
        .fields
        .iter()
        .filter(|f| f.cardinality == Cardinality::Required)
        .collect::<Vec<_>>();
    if !required.is_empty() {
        writeln!(
            out,
            "  if ({}) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;",
            required
                .iter()
                .map(|f| format!("!{}", access(plan, f, "value", false, true)))
                .collect::<Vec<_>>()
                .join(" || ")
        )
        .unwrap();
    }
    let mut sizes = vec![format!("{presence}U")];
    for field in &plan.message.fields {
        let v = access(plan, field, "value", false, false);
        let size = if plan.varuint32(field) {
            format!("{p}vsize32({v})")
        } else {
            format!("{}U", width(&field.ty))
        };
        sizes.push(if field.cardinality == Cardinality::Required {
            size
        } else {
            format!(
                "({} ? {size} : 0U)",
                access(plan, field, "value", false, true)
            )
        });
    }
    writeln!(out, "  *size = {};\n  return WL_CODEC_OK;\n}}\nstatic void {name}_clear(void *value) {{\n  if (value == NULL) return;", sizes.join(" + ")).unwrap();
    for field in &plan.message.fields {
        let (_, _, unsigned, _, _) = field_descriptor_data(field);
        let default = match field.ty {
            ResolvedType::Uint32 | ResolvedType::Fixed32 => format!("UINT32_C({unsigned})"),
            ResolvedType::Fixed64 => format!("UINT64_C({unsigned})"),
            _ => "0.0".into(),
        };
        writeln!(
            out,
            "  {} = false;\n  {} = {default};",
            access(plan, field, "value", true, true),
            access(plan, field, "value", true, false)
        )
        .unwrap();
    }
    writeln!(out,"}}\nstatic size_t {name}_encoded_size(const void *value) {{ size_t size; return {name}_wlc_detail_measure(value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\nstatic wl_codec_status_t {name}_encode(const void *value, uint8_t *out, size_t capacity, size_t *length) {{\n  size_t size;\n  wl_codec_status_t status = {name}_wlc_detail_measure(value, &size);\n  if (status != WL_CODEC_OK) return status;\n  if (length == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  if (capacity < size) return WL_CODEC_ERR_CAPACITY;\n  uint8_t *cursor = out;").unwrap();
    if presence != 0 {
        let bits = plan
            .message
            .fields
            .iter()
            .filter(|f| f.cardinality == Cardinality::Optional)
            .enumerate()
            .map(|(bit, f)| {
                format!(
                    "({} ? {}U : 0U)",
                    access(plan, f, "value", false, true),
                    1_u16 << bit
                )
            })
            .collect::<Vec<_>>();
        writeln!(out, "  *cursor++ = (uint8_t)({});", bits.join(" | ")).unwrap();
    }
    for field in &plan.message.fields {
        writeln!(
            out,
            "  if ({}) {{",
            access(plan, field, "value", false, true)
        )
        .unwrap();
        emit_bits(
            &mut out,
            field,
            &access(plan, field, "value", false, false),
            "    ",
        );
        let put = if plan.varuint32(field) {
            format!("{p}putv32")
        } else {
            format!("{p}put{}", width(&field.ty) * 8)
        };
        writeln!(out, "    {put}(&cursor, bits);\n  }}").unwrap();
    }
    writeln!(out,"  *length = size;\n  return WL_CODEC_OK;\n}}\nstatic wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, void *out) {{\n  if (out == NULL || (length != 0U && input == NULL)) return WL_CODEC_ERR_INVALID_VALUE;").unwrap();
    if presence != 0 {
        writeln!(out, "  if (length == 0U) return WL_CODEC_ERR_MALFORMED;").unwrap();
        if optional < 8 {
            writeln!(
                out,
                "  if ((input[0] >> {optional}U) != 0U) return WL_CODEC_ERR_MALFORMED;"
            )
            .unwrap();
        }
    }
    writeln!(out, "  {name}_clear(out);\n  size_t at = {presence}U;").unwrap();
    let mut bit = 0;
    for field in &plan.message.fields {
        if field.cardinality == Cardinality::Optional {
            writeln!(out, "  if ((input[0] & {}U) != 0U) {{", 1_u16 << bit).unwrap();
            bit += 1;
        } else {
            out.push_str("  {\n");
        }
        let v = access(plan, field, "out", true, false);
        if plan.varuint32(field) {
            writeln!(out,"    uint32_t bits;\n    wl_codec_status_t status = {p}getv32(input, length, &at, &bits);\n    if (status != WL_CODEC_OK) return status;\n    {v} = bits;").unwrap();
        } else {
            let bytes = width(&field.ty);
            let bits = bytes * 8;
            writeln!(out,"    if (length - at < {bytes}U) return WL_CODEC_ERR_MALFORMED;\n    uint{bits}_t bits = 0U;\n    for (size_t i = 0U; i < {bytes}U; ++i) bits = (uint{bits}_t)((bits << 8U) | input[at++]);").unwrap();
            if matches!(field.ty, ResolvedType::Float32 | ResolvedType::Float64) {
                writeln!(out, "    memcpy(&{v}, &bits, sizeof(bits));").unwrap();
            } else {
                writeln!(out, "    {v} = bits;").unwrap();
            }
        }
        writeln!(
            out,
            "    {} = true;\n  }}",
            access(plan, field, "out", true, true)
        )
        .unwrap();
    }
    out.push_str("  return at == length ? WL_CODEC_OK : WL_CODEC_ERR_MALFORMED;\n}\n");
    if fingerprint {
        let maximum = presence
            + plan
                .message
                .fields
                .iter()
                .map(|f| if plan.varuint32(f) { 5 } else { width(&f.ty) })
                .sum::<usize>();
        writeln!(out,"static wl_codec_status_t {name}_wlc_detail_fingerprint(const void *value, uint64_t *hash, size_t *length) {{\n  uint8_t bytes[{maximum}];\n  size_t size;\n  wl_codec_status_t status = {name}_encode(value, bytes, sizeof(bytes), &size);\n  if (status != WL_CODEC_OK) return status;\n  {p}hash_state_t state = {{*hash, 0U, WL_CODEC_OK}};\n  {p}hash_copy_span(&state, bytes, size);\n  *hash = state.hash; *length = state.length;\n  return WL_CODEC_OK;\n}}\n").unwrap();
    }
    out
}
