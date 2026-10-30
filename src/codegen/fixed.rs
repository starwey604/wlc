// SPDX-License-Identifier: Apache-2.0
//! Direct codecs for small scalar messages. The public wire remains TLV.
use std::fmt::Write;

use super::Symbols;
use super::{
    c_identifier, c_type, descriptor::field_descriptor_data, plan::MessagePlan, type_name,
};
use crate::{ast::Cardinality, semantic::ResolvedType};

pub(super) fn supports(plan: &MessagePlan<'_>) -> bool {
    !plan.message.fields.is_empty()
        && plan.message.fields.len() <= 8
        && plan.message.fields.iter().all(|field| {
            matches!(
                field.cardinality,
                Cardinality::Required | Cardinality::Optional
            ) && matches!(
                field.ty,
                ResolvedType::Uint32
                    | ResolvedType::Fixed32
                    | ResolvedType::Fixed64
                    | ResolvedType::Float32
                    | ResolvedType::Float64
            )
        })
}

pub(super) fn width(ty: &ResolvedType) -> usize {
    match ty {
        ResolvedType::Uint32 | ResolvedType::Fixed32 | ResolvedType::Float32 => 4,
        _ => 8,
    }
}

pub(super) fn same_shape(a: &MessagePlan<'_>, b: &MessagePlan<'_>) -> bool {
    a.profile == b.profile
        && a.message.fields.len() == b.message.fields.len()
        && a.message
            .fields
            .iter()
            .zip(&b.message.fields)
            .all(|(a, b)| {
                a.number == b.number
                    && a.cardinality == b.cardinality
                    && a.ty == b.ty
                    && a.default == b.default
                    && a.max_length == b.max_length
            })
}

pub(super) fn shape_name(plan: &MessagePlan<'_>, index: usize, symbols: &Symbols) -> String {
    let kind = if plan.profile == crate::codec_plan::CodecProfile::PositionalV1 {
        "positional_scalar_shape"
    } else {
        "scalar_shape"
    };
    format!("{}{kind}_{index}", symbols.helper)
}

pub(super) fn shape_source(
    plan: &MessagePlan<'_>,
    index: usize,
    fingerprint: bool,
    symbols: &Symbols,
) -> String {
    let name = shape_name(plan, index, symbols);
    if plan.profile == crate::codec_plan::CodecProfile::PositionalV1 {
        super::positional_fixed::source(plan, &name, fingerprint, symbols)
    } else {
        source(plan, &name, fingerprint, symbols)
    }
}

// Shared shapes use primitive lvalues and schema offsets, never casts between
// distinct message structs. Names and accesses are selected before C emission.
pub(super) fn access(
    plan: &MessagePlan<'_>,
    field: &crate::semantic::FieldSymbol,
    base: &str,
    mutable: bool,
    presence: bool,
) -> String {
    let ty = type_name(&plan.message.name);
    let f = c_identifier(&field.name);
    let member = if presence { format!("has_{f}") } else { f };
    let primitive = if presence {
        "bool".into()
    } else {
        c_type(&field.ty)
    };
    let qualifier = if mutable { "" } else { "const " };
    format!(
        "(*({qualifier}{primitive} *)(({qualifier}uint8_t *){base} + offsetof({ty}_t, {member})))"
    )
}

pub(super) fn wrappers(
    plan: &MessagePlan<'_>,
    index: usize,
    fingerprint: bool,
    symbols: &Symbols,
) -> String {
    let name = symbols.message(plan.message);
    let ty = type_name(&plan.message.name);
    let storage = symbols.storage;
    let shape = shape_name(plan, index, symbols);
    let mut out = String::new();
    writeln!(out, "static inline wl_codec_status_t {name}_wlc_detail_measure(const {ty}_t *value, size_t *size) {{\n  (void){name}_desc;\n  return {shape}_wlc_detail_measure(value, size);\n}}\n{storage}void {name}_clear({ty}_t *value) {{ {shape}_clear(value); }}\n{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ return {shape}_encoded_size(value); }}\n{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t capacity, size_t *length) {{ return {shape}_encode(value, out, capacity, length); }}\n{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {ty}_t *out) {{ return {shape}_decode(input, length, out); }}\n").unwrap();
    if fingerprint {
        writeln!(out, "{storage}wl_codec_status_t {name}_wlc_detail_fingerprint(const {ty}_t *value, uint64_t *hash, size_t *length) {{ return {shape}_wlc_detail_fingerprint(value, hash, length); }}").unwrap();
    }
    out
}

fn source(plan: &MessagePlan<'_>, name: &str, fingerprint: bool, symbols: &Symbols) -> String {
    let p = &symbols.helper;
    let storage = "static ";
    let mut out = format!(
        "/* Schema-specialized scalars; ordinary Schema-v1 TLV bytes. */\nstatic inline wl_codec_status_t {name}_wlc_detail_measure(const void *value, size_t *size) {{\n  if (value == NULL || size == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n"
    );
    let mut required = Vec::new();
    let mut size = Vec::new();
    for (field, key) in plan.message.fields.iter().zip(&plan.keys) {
        let v = access(plan, field, "value", false, false);
        let vh = access(plan, field, "value", false, true);
        let field_size = if plan.varuint32(field) {
            format!("({}U + {p}vsize32({v}))", key.length)
        } else {
            format!("{}U", key.length + width(&field.ty))
        };
        if field.cardinality == Cardinality::Required {
            required.push(format!("!{vh}"));
            size.push(field_size);
        } else {
            size.push(format!("({vh} ? {field_size} : 0U)"));
        }
    }
    if !required.is_empty() {
        writeln!(
            out,
            "  if ({}) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;",
            required.join(" || ")
        )
        .unwrap();
    }
    writeln!(out, "  *size = {};\n  return WL_CODEC_OK;\n}}\n\n{storage}void {name}_clear(void *value) {{\n  if (value == NULL) return;", size.join(" + ")).unwrap();
    for field in &plan.message.fields {
        let mv = access(plan, field, "value", true, false);
        let mh = access(plan, field, "value", true, true);
        let (_, _, unsigned, _, _) = field_descriptor_data(field);
        let default = match field.ty {
            ResolvedType::Uint32 | ResolvedType::Fixed32 => format!("UINT32_C({unsigned})"),
            ResolvedType::Fixed64 => format!("UINT64_C({unsigned})"),
            _ => "0.0".to_owned(),
        };
        writeln!(out, "  {mh} = false;\n  {mv} = {default};").unwrap();
    }
    writeln!(out, "}}\n\n{storage}size_t {name}_encoded_size(const void *value) {{\n  size_t size;\n  return {name}_wlc_detail_measure(value, &size) == WL_CODEC_OK ? size : SIZE_MAX;\n}}\n\n{storage}wl_codec_status_t {name}_encode(const void *value, uint8_t *out, size_t capacity, size_t *length) {{\n  size_t size;\n  wl_codec_status_t status = {name}_wlc_detail_measure(value, &size);\n  if (status != WL_CODEC_OK) return status;\n  if (length == NULL || (size != 0U && out == NULL)) return WL_CODEC_ERR_INVALID_VALUE;\n  if (capacity < size) return WL_CODEC_ERR_CAPACITY;\n  uint8_t *cursor = out;").unwrap();
    emit_scalar_fields(&mut out, plan, symbols, false);
    writeln!(out, "  *length = size;\n  return WL_CODEC_OK;\n}}\n\n{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, void *out) {{\n  if (out == NULL || (length != 0U && input == NULL)) return WL_CODEC_ERR_INVALID_VALUE;\n  {name}_clear(out);\n  for (size_t at = 0U; at < length;) {{\n    uint32_t key;\n    wl_codec_status_t status;\n    if (input[at] < 128U) key = input[at++];\n    else {{\n      uint64_t wide;\n      status = {p}getv(input, length, &at, &wide);\n      if (status != WL_CODEC_OK) return status;\n      if (wide > UINT32_C(0x7ffff)) return WL_CODEC_ERR_MALFORMED;\n      key = (uint32_t)wide;\n    }}\n    uint32_t number = key >> 3U;\n    uint8_t wire = (uint8_t)(key & 7U);\n    if (number == 0U || number > 65535U ||\n        (wire != 0U && wire != 1U && wire != 2U && wire != 5U))\n      return WL_CODEC_ERR_MALFORMED;\n    switch ((uint16_t)number) {{").unwrap();
    for (field, key) in plan.message.fields.iter().zip(&plan.keys) {
        let o = access(plan, field, "out", true, false);
        let oh = access(plan, field, "out", true, true);
        let bytes = width(&field.ty);
        if plan.varuint32(field) {
            writeln!(out, "      case {}U: {{\n        if (wire != 0U) return WL_CODEC_ERR_WIRE_TYPE;\n        if ({oh}) return WL_CODEC_ERR_DUPLICATE_FIELD;\n        uint32_t bits;\n        status = {p}getv32(input, length, &at, &bits);\n        if (status != WL_CODEC_OK) return status;\n        {o} = bits;\n        {oh} = true;\n        break;\n      }}", field.number).unwrap();
            continue;
        }
        writeln!(out, "      case {}U: {{\n        if (wire != {}U) return WL_CODEC_ERR_WIRE_TYPE;\n        if ({oh}) return WL_CODEC_ERR_DUPLICATE_FIELD;\n        if (length - at < {bytes}U) return WL_CODEC_ERR_MALFORMED;", field.number, key.wire).unwrap();
        let terms = (0..bytes)
            .map(|i| {
                format!(
                    "((uint{}_t)input[at + {i}U] << {}U)",
                    bytes * 8,
                    (bytes - i - 1) * 8
                )
            })
            .collect::<Vec<_>>();
        writeln!(
            out,
            "        uint{}_t bits = {};",
            bytes * 8,
            terms.join(" | ")
        )
        .unwrap();
        if matches!(field.ty, ResolvedType::Float32 | ResolvedType::Float64) {
            writeln!(out, "        memcpy(&{o}, &bits, sizeof(bits));").unwrap();
        } else {
            writeln!(out, "        {o} = bits;").unwrap();
        }
        writeln!(
            out,
            "        {oh} = true;\n        at += {bytes}U;\n        break;\n      }}"
        )
        .unwrap();
    }
    writeln!(out, "      default:\n        status = {p}skip(wire, input, length, &at);\n        if (status != WL_CODEC_OK) return status;\n        break;\n    }}\n  }}").unwrap();
    if !required.is_empty() {
        writeln!(
            out,
            "  if ({}) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;",
            plan.message
                .fields
                .iter()
                .filter(|f| f.cardinality == Cardinality::Required)
                .map(|f| format!("!{}", access(plan, f, "out", true, true)))
                .collect::<Vec<_>>()
                .join(" || ")
        )
        .unwrap();
    }
    writeln!(out, "  return WL_CODEC_OK;\n}}\n").unwrap();
    if fingerprint {
        let maximum = plan
            .message
            .fields
            .iter()
            .zip(&plan.keys)
            .map(|(field, key)| {
                key.length
                    + if plan.varuint32(field) {
                        5
                    } else {
                        width(&field.ty)
                    }
            })
            .sum::<usize>();
        writeln!(out, "/* Generator-private: unmodified successful decode, caller supplies domain seed.\n * Reuse the direct encoder, then one byte loop; no per-field FNV expansion. */\n{storage}wl_codec_status_t {name}_wlc_detail_fingerprint(const void *value, uint64_t *hash, size_t *length) {{\n  uint8_t bytes[{maximum}];\n  size_t size;\n  wl_codec_status_t status = {name}_encode(value, bytes, sizeof(bytes), &size);\n  if (status != WL_CODEC_OK) return status;\n  {p}hash_state_t state = {{*hash, 0U, WL_CODEC_OK}};\n  {p}hash_copy_span(&state, bytes, size);\n  *hash = state.hash;\n  *length = state.length;\n  return WL_CODEC_OK;\n}}\n").unwrap();
    }
    out
}

fn emit_scalar_fields(
    out: &mut String,
    plan: &MessagePlan<'_>,
    symbols: &Symbols,
    direct_fixed: bool,
) {
    let p = &symbols.helper;
    for (field, key) in plan.message.fields.iter().zip(&plan.keys) {
        let v = access(plan, field, "value", false, false);
        let vh = access(plan, field, "value", false, true);
        writeln!(out, "  if ({vh}) {{").unwrap();
        for b in &key.bytes[..key.length] {
            writeln!(out, "    *cursor++ = {b}U;").unwrap();
        }
        emit_bits(out, field, &v, "    ");
        let put = if plan.varuint32(field) {
            format!("{p}putv32")
        } else {
            format!("{p}put{}", width(&field.ty) * 8)
        };
        if direct_fixed && !plan.varuint32(field) {
            for byte in (0..width(&field.ty)).rev() {
                writeln!(out, "    *cursor++ = (uint8_t)(bits >> {}U);", byte * 8).unwrap();
            }
            out.push_str("  }\n");
        } else {
            writeln!(out, "    {put}(&cursor, bits);\n  }}").unwrap();
        }
    }
}

// The runtime consumes the same canonical field/key plan as the ordinary
// fingerprint. It never fingerprints the managed wire profile. Keeping this
// small byte emitter at the validated request consumer avoids a second codec
// traversal and a cross-translation-unit call, without expanding the hash loop.
pub(super) fn canonical_fingerprint(
    plan: &MessagePlan<'_>,
    name: &str,
    symbols: &Symbols,
) -> String {
    debug_assert!(supports(plan));
    debug_assert_eq!(plan.profile, crate::codec_plan::CodecProfile::SchemaV1);
    let ty = type_name(&plan.message.name);
    let maximum: usize = plan
        .message
        .fields
        .iter()
        .zip(&plan.keys)
        .map(|(field, key)| {
            key.length
                + if plan.varuint32(field) {
                    5
                } else {
                    width(&field.ty)
                }
        })
        .sum();
    let mut out = format!(
        "/* Private canonical Schema-v1 fingerprint, after successful decode. */\nstatic inline wl_codec_status_t {name}(const {ty}_t *value, uint64_t *hash, size_t *length) {{\n  uint8_t bytes[{maximum}];\n  uint8_t *cursor = bytes;\n"
    );
    emit_scalar_fields(&mut out, plan, symbols, true);
    out.push_str("  size_t size = (size_t)(cursor - bytes);\n  uint64_t h = *hash;\n  for (size_t i = 0U; i < size; ++i) {\n    h = (h ^ bytes[i]) * UINT64_C(0x00000100000001b3);\n  }\n  *hash = h;\n  *length = size;\n  return WL_CODEC_OK;\n}\n\n");
    out
}

pub(super) fn emit_bits(
    out: &mut String,
    field: &crate::semantic::FieldSymbol,
    value: &str,
    indent: &str,
) {
    let bits = width(&field.ty) * 8;
    if matches!(field.ty, ResolvedType::Float32 | ResolvedType::Float64) {
        writeln!(
            out,
            "{indent}uint{bits}_t bits;\n{indent}memcpy(&bits, &{value}, sizeof(bits));"
        )
        .unwrap();
    } else {
        writeln!(out, "{indent}uint{bits}_t bits = {value};").unwrap();
    }
}
