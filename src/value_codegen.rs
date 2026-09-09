//! Pointer-free business values for schemas with a finite static bound.
//! Keep the existing descriptor codec as the single wire-format implementation.

use std::{collections::HashMap, fmt::Write};

use crate::{
    ast::Cardinality,
    codegen::{c_identifier, c_type, field_descriptor_data, type_name, upper_snake},
    semantic::{FieldDefault, FieldSymbol, MessageSymbol, ResolvedType},
};

pub(crate) fn header(
    out: &mut String,
    messages: &[&MessageSymbol],
    maxima: &HashMap<u16, Option<u64>>,
) {
    out.push_str("/* Self-owning business values. Assignment copies all data; no destroy is needed.\n * String lengths are bytes (embedded NUL is allowed); data[length] is a\n * convenience terminator after clear/decode/from_view, not part of the wire.\n * Views borrow their source. Conversion input/output must not overlap.\n * Failed conversions/decodes leave output unchanged. */\n");
    for message in messages {
        let name = type_name(&message.name);
        let prefix = upper_snake(&message.name);
        let bounded = maxima.get(&message.id).copied().flatten().is_some();
        writeln!(out, "#define {prefix}_HAS_VALUE {}", u8::from(bounded)).unwrap();
        if !bounded {
            continue;
        }
        writeln!(out, "typedef struct {{").unwrap();
        if message.fields.is_empty() {
            out.push_str("  uint8_t _empty;\n");
        }
        for field in &message.fields {
            let field_name = c_identifier(&field.name);
            writeln!(out, "  bool has_{field_name};").unwrap();
            match field.cardinality {
                Cardinality::Packed(count) | Cardinality::RequiredPacked(count) => {
                    writeln!(out, "  {} {field_name}[{count}];", c_type(&field.ty)).unwrap();
                }
                Cardinality::Optional | Cardinality::Required => {
                    match &field.ty {
                        ResolvedType::Bytes | ResolvedType::String => {
                            let bound = u64::from(field.max_length.unwrap());
                            let (element, storage) = if field.ty == ResolvedType::String {
                                ("char", bound + 1)
                            } else {
                                ("uint8_t", bound.max(1))
                            };
                            writeln!(out, "  struct {{ size_t length; {element} data[{storage}]; }} {field_name};").unwrap();
                        }
                        ResolvedType::Message { name, .. } => {
                            writeln!(out, "  {}_value_t {field_name};", type_name(name)).unwrap();
                        }
                        _ => writeln!(out, "  {} {field_name};", c_type(&field.ty)).unwrap(),
                    }
                }
                Cardinality::Repeated => unreachable!("unbounded repeated has no value"),
            }
        }
        writeln!(out, "}} {name}_value_t;\n#define {prefix}_VALUE_SIZE (sizeof({name}_value_t))\nvoid {name}_value_clear({name}_value_t *value);\nsize_t {name}_value_encoded_size(const {name}_value_t *value);\nwl_codec_status_t {name}_value_encode(const {name}_value_t *value, uint8_t *out, size_t capacity, size_t *length);\nwl_codec_status_t {name}_value_decode(const uint8_t *input, size_t length, {name}_value_t *out);\n").unwrap();
    }
}

pub(crate) fn source(
    out: &mut String,
    messages: &[&MessageSymbol],
    maxima: &HashMap<u16, Option<u64>>,
) {
    for message in messages {
        if maxima.get(&message.id).copied().flatten().is_none() {
            continue;
        }
        let name = type_name(&message.name);
        // The outermost operation zeros storage once. Nested defaults/copies
        // never clear the same subtree again or build a full temporary view.
        writeln!(
            out,
            "static void {name}_value_defaults({name}_value_t *out) {{\n  (void)out;"
        )
        .unwrap();
        for field in &message.fields {
            emit_default(out, field);
        }
        writeln!(out, "}}\n\nstatic void {name}_value_copy_fields(const {name}_t *view, {name}_value_t *out) {{").unwrap();
        if message.fields.is_empty() {
            out.push_str("  (void)view; (void)out;\n");
        }
        for field in &message.fields {
            let f = c_identifier(&field.name);
            writeln!(out, "  out->has_{f} = view->has_{f};").unwrap();
            match field.cardinality {
                Cardinality::Packed(_) | Cardinality::RequiredPacked(_) => {
                    writeln!(
                        out,
                        "  if (view->has_{f}) memcpy(out->{f}, view->{f}, sizeof(out->{f}));"
                    )
                    .unwrap();
                }
                _ => match &field.ty {
                    ResolvedType::String | ResolvedType::Bytes => {
                        writeln!(out, "  if (view->has_{f}) {{\n    out->{f}.length = view->{f}.length;\n    if (view->{f}.length != 0U) memcpy(out->{f}.data, view->{f}.data, view->{f}.length);\n  }} else {{").unwrap();
                        emit_default(out, field);
                        out.push_str("  }\n");
                    }
                    ResolvedType::Message { name: child, .. } => {
                        let child = type_name(child);
                        writeln!(out, "  if (view->has_{f}) {child}_value_copy_fields(&view->{f}, &out->{f});\n  else {child}_value_defaults(&out->{f});").unwrap();
                    }
                    _ => writeln!(
                        out,
                        "  out->{f} = view->has_{f} ? view->{f} : {};",
                        scalar_default(field)
                    )
                    .unwrap(),
                },
            }
        }
        writeln!(out, "}}\n\n/* Generator-private: input is an unmodified successful decode, or has been measured. */\nvoid {name}_wlc_detail_value_copy(const {name}_t *view, {name}_value_t *out) {{\n  memset(out, 0, sizeof(*out));\n  {name}_value_copy_fields(view, out);\n}}\n\nvoid {name}_value_clear({name}_value_t *value) {{\n  if (value == NULL) return;\n  memset(value, 0, sizeof(*value));\n  {name}_value_defaults(value);\n}}\n\nwl_codec_status_t {name}_value_from_view(const {name}_t *view, {name}_value_t *out) {{\n  size_t size;\n  wl_codec_status_t status;\n  if (view == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = wlc_measure(&{name}_desc, view, &size);\n  if (status != WL_CODEC_OK) return status;\n  {name}_wlc_detail_value_copy(view, out);\n  return WL_CODEC_OK;\n}}\n").unwrap();
        writeln!(out, "/* Private conversion does not re-validate each nested subtree. */\nstatic wl_codec_status_t {name}_value_borrow(const {name}_value_t *value, {name}_t *out) {{\n  {name}_clear(out);").unwrap();
        if message.fields.is_empty() {
            out.push_str("  (void)value;\n");
        }
        for field in &message.fields {
            let f = c_identifier(&field.name);
            writeln!(
                out,
                "  out->has_{f} = value->has_{f};\n  if (value->has_{f}) {{"
            )
            .unwrap();
            match field.cardinality {
                Cardinality::Packed(_) | Cardinality::RequiredPacked(_) => {
                    writeln!(out, "    memcpy(out->{f}, value->{f}, sizeof(out->{f}));").unwrap();
                }
                _ => match &field.ty {
                    ResolvedType::String | ResolvedType::Bytes => {
                        writeln!(out, "    if (value->{f}.length > {}U) return WL_CODEC_ERR_INVALID_VALUE;\n    out->{f}.length = value->{f}.length;\n    out->{f}.data = value->{f}.data;", field.max_length.unwrap()).unwrap();
                    }
                    ResolvedType::Message { name: child, .. } => {
                        writeln!(out, "    wl_codec_status_t status = {}_value_borrow(&value->{f}, &out->{f});\n    if (status != WL_CODEC_OK) return status;", type_name(child)).unwrap();
                    }
                    _ => writeln!(out, "    out->{f} = value->{f};").unwrap(),
                },
            }
            out.push_str("  }\n");
        }
        writeln!(out, "  return WL_CODEC_OK;\n}}\n\nwl_codec_status_t {name}_value_to_view(const {name}_value_t *value, {name}_t *out) {{\n  {name}_t view;\n  size_t size;\n  wl_codec_status_t status;\n  if (value == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  if (status == WL_CODEC_OK) status = wlc_measure(&{name}_desc, &view, &size);\n  if (status != WL_CODEC_OK) return status;\n  *out = view;\n  return WL_CODEC_OK;\n}}\n\nsize_t {name}_value_encoded_size(const {name}_value_t *value) {{\n  {name}_t view;\n  if (value == NULL || {name}_value_borrow(value, &view) != WL_CODEC_OK) return SIZE_MAX;\n  return {name}_encoded_size(&view);\n}}\n\nwl_codec_status_t {name}_value_encode(const {name}_value_t *value, uint8_t *out, size_t capacity, size_t *length) {{\n  {name}_t view;\n  wl_codec_status_t status;\n  if (value == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  return status == WL_CODEC_OK ? {name}_encode(&view, out, capacity, length) : status;\n}}\n\nwl_codec_status_t {name}_value_decode(const uint8_t *input, size_t length, {name}_value_t *out) {{\n  {name}_t view;\n  wl_codec_status_t status;\n  if (out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_decode(input, length, &view);\n  if (status != WL_CODEC_OK) return status;\n  {name}_wlc_detail_value_copy(&view, out);\n  return WL_CODEC_OK;\n}}\n").unwrap();
    }
}

fn scalar_default(field: &FieldSymbol) -> String {
    let (_, signed, unsigned, _, _) = field_descriptor_data(field);
    match field.ty {
        ResolvedType::Int8
        | ResolvedType::Int16
        | ResolvedType::Int32
        | ResolvedType::Int64
        | ResolvedType::Enum { .. } => signed,
        ResolvedType::Uint64 | ResolvedType::Fixed64 => format!("UINT64_C({unsigned})"),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => format!("UINT32_C({unsigned})"),
        _ => unsigned,
    }
}

// Called only on already-zeroed owned storage. Never follow an absent view's
// pointers, and never clear a nested array a second time to apply its defaults.
fn emit_default(out: &mut String, field: &FieldSymbol) {
    if matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    ) {
        return;
    }
    let f = c_identifier(&field.name);
    match &field.ty {
        ResolvedType::Message { name, .. } => {
            writeln!(out, "  {}_value_defaults(&out->{f});", type_name(name)).unwrap();
        }
        ResolvedType::String => {
            if let Some(FieldDefault::String(value)) = &field.default {
                let (_, _, _, literal, _) = field_descriptor_data(field);
                writeln!(out, "  out->{f}.length = {}U;", value.len()).unwrap();
                if !value.is_empty() {
                    writeln!(out, "  memcpy(out->{f}.data, {literal}, {}U);", value.len()).unwrap();
                }
            }
        }
        ResolvedType::Bytes => {}
        _ => writeln!(out, "  out->{f} = {};", scalar_default(field)).unwrap(),
    }
}
