//! Pointer-free business values for schemas with a finite static bound.
//! Reuse the descriptor codec traversal and primitive wire readers.

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
    codecs: &crate::codec_plan::CodecPlan<'_>,
    symbols: &crate::codegen::Symbols,
) {
    let p = &symbols.helper;
    let storage = symbols.storage;
    let tagged = crate::codegen::tagged_dependencies(codecs, true);
    if messages
        .iter()
        .any(|m| maxima.get(&m.id).copied().flatten().is_some())
    {
        out.push_str(&crate::codegen::owned_decode_source(
            messages, codecs, symbols,
        ));
        for message in messages
            .iter()
            .filter(|m| maxima.get(&m.id).copied().flatten().is_some())
        {
            writeln!(
                out,
                "static const {p}value_desc_t {}_value_desc;",
                symbols.message(message)
            )
            .unwrap();
        }
        for &id in &tagged {
            writeln!(
                out,
                "static const {p}value_desc_t {}_value_desc;",
                crate::codegen::tagged_name(id, symbols)
            )
            .unwrap();
        }
    }
    for message in messages {
        if maxima.get(&message.id).copied().flatten().is_none() {
            continue;
        }
        let ty = type_name(&message.name);
        let name = symbols.message(message);
        // The outermost operation zeros storage once. Nested defaults/copies
        // never clear the same subtree again or build a full temporary view.
        writeln!(
            out,
            "static void {name}_value_defaults({ty}_value_t *out) {{\n  (void)out;"
        )
        .unwrap();
        for field in &message.fields {
            emit_default(out, field, &symbols.prefix);
        }
        writeln!(out, "}}\n\nstatic void {name}_value_decode_defaults(void *out) {{ {name}_value_defaults(out); }}\n").unwrap();
        crate::codegen::owned_descriptor(
            out,
            message,
            codecs,
            symbols,
            tagged.contains(&message.id),
        );
        writeln!(
            out,
            "static wl_codec_status_t {name}_value_borrow(const {ty}_value_t *, {ty}_t *);"
        )
        .unwrap();
        let positional =
            codecs.messages[&message.id].profile == crate::codec_plan::CodecProfile::PositionalV1;
        let scalar_decode = crate::codegen::scalar_value_decode(message, codecs, symbols);
        let value_codec = if scalar_decode.is_some() {
            format!(
                "{storage}size_t {name}_value_encoded_size(const {ty}_value_t *value) {{ return {name}_wlc_detail_encoded_size_fields(value); }}\n\n{storage}wl_codec_status_t {name}_value_encode(const {ty}_value_t *value, uint8_t *out, size_t capacity, size_t *length) {{ return {name}_wlc_detail_encode_fields(value, out, capacity, length); }}\n\n"
            )
        } else {
            format!(
                "{storage}size_t {name}_value_encoded_size(const {ty}_value_t *value) {{\n  {ty}_t view;\n  if (value == NULL || {name}_value_borrow(value, &view) != WL_CODEC_OK) return SIZE_MAX;\n  return {name}_encoded_size(&view);\n}}\n\n{storage}wl_codec_status_t {name}_value_encode(const {ty}_value_t *value, uint8_t *out, size_t capacity, size_t *length) {{\n  {ty}_t view;\n  wl_codec_status_t status;\n  if (value == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  return status == WL_CODEC_OK ? {name}_encode(&view, out, capacity, length) : status;\n}}\n\n"
            )
        };
        let prepare_value = if scalar_decode.is_some() {
            "  memset(&value, 0, sizeof(value));\n"
        } else {
            ""
        };
        if let Some(scalar) = scalar_decode {
            out.push_str(&scalar);
        } else if positional {
            out.push_str(&crate::codegen::positional::owned_decode(
                message,
                &codecs.messages[&message.id],
                symbols,
            ));
        } else {
            writeln!(out, "static inline wl_codec_status_t {name}_wlc_detail_value_decode(const uint8_t *input, size_t length, {ty}_value_t *out) {{ return {p}value_decode(&{name}_value_desc.wire, input, length, out); }}\n").unwrap();
        }
        writeln!(
            out,
            "static void {name}_value_copy_fields(const {ty}_t *view, {ty}_value_t *out) {{"
        )
        .unwrap();
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
                        emit_default(out, field, &symbols.prefix);
                        out.push_str("  }\n");
                    }
                    ResolvedType::Message { name: child, .. } => {
                        let child = format!("{}{}", symbols.prefix, type_name(child));
                        writeln!(out, "  if (view->has_{f}) {child}_value_copy_fields(&view->{f}, &out->{f});\n  else {child}_value_defaults(&out->{f});").unwrap();
                    }
                    ResolvedType::Float32 | ResolvedType::Float64 => {
                        writeln!(out, "  if (view->has_{f}) memcpy(&out->{f}, &view->{f}, sizeof(out->{f}));\n  else out->{f} = 0.0;").unwrap();
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
        writeln!(out, "}}\n\n/* Generator-private: input is an unmodified successful decode, or has been measured. */\n{storage}void {name}_wlc_detail_value_copy(const {ty}_t *view, {ty}_value_t *out) {{\n  memset(out, 0, sizeof(*out));\n  {name}_value_copy_fields(view, out);\n}}\n\n{storage}void {name}_value_clear({ty}_value_t *value) {{\n  if (value == NULL) return;\n  memset(value, 0, sizeof(*value));\n  {name}_value_defaults(value);\n}}\n\n{storage}wl_codec_status_t {name}_value_from_view(const {ty}_t *view, {ty}_value_t *out) {{\n  size_t size;\n  wl_codec_status_t status;\n  if (view == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_wlc_detail_measure(view, &size);\n  if (status != WL_CODEC_OK) return status;\n  {name}_wlc_detail_value_copy(view, out);\n  return WL_CODEC_OK;\n}}\n").unwrap();
        writeln!(out, "/* Private conversion does not re-validate each nested subtree. */\nstatic wl_codec_status_t {name}_value_borrow(const {ty}_value_t *value, {ty}_t *out) {{\n  {name}_clear(out);").unwrap();
        if message.fields.is_empty() {
            out.push_str("  (void)value;\n  out->_empty = 0U;\n");
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
                        writeln!(out, "    wl_codec_status_t status = {}_value_borrow(&value->{f}, &out->{f});\n    if (status != WL_CODEC_OK) return status;", format_args!("{}{}", symbols.prefix, type_name(child))).unwrap();
                    }
                    ResolvedType::Float32 | ResolvedType::Float64 => {
                        writeln!(out, "    memcpy(&out->{f}, &value->{f}, sizeof(out->{f}));")
                            .unwrap();
                    }
                    _ => writeln!(out, "    out->{f} = value->{f};").unwrap(),
                },
            }
            out.push_str("  }\n");
        }
        writeln!(out, "  return WL_CODEC_OK;\n}}\n\n{storage}wl_codec_status_t {name}_value_to_view(const {ty}_value_t *value, {ty}_t *out) {{\n  {ty}_t view;\n  size_t size;\n  wl_codec_status_t status;\n  if (value == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  if (status == WL_CODEC_OK) status = {name}_wlc_detail_measure(&view, &size);\n  if (status != WL_CODEC_OK) return status;\n  *out = view;\n  return WL_CODEC_OK;\n}}\n\n{value_codec}{storage}wl_codec_status_t {name}_value_decode(const uint8_t *input, size_t length, {ty}_value_t *out) {{\n  {ty}_value_t value;\n  wl_codec_status_t status;\n  if (out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n{prepare_value}  status = {name}_wlc_detail_value_decode(input, length, &value);\n  if (status != WL_CODEC_OK) return status;\n  *out = value;\n  return WL_CODEC_OK;\n}}\n").unwrap();
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
fn emit_default(out: &mut String, field: &FieldSymbol, prefix: &str) {
    if matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    ) {
        return;
    }
    let f = c_identifier(&field.name);
    match &field.ty {
        ResolvedType::Message { name, .. } => {
            writeln!(
                out,
                "  {prefix}{}_value_defaults(&out->{f});",
                type_name(name)
            )
            .unwrap();
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
