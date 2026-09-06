//! Pointer-free business values for schemas with a finite static bound.
//! Keep the existing descriptor codec as the single wire-format implementation.

use std::{collections::HashMap, fmt::Write};

use crate::{
    ast::Cardinality,
    codegen::{c_identifier, c_type, type_name, upper_snake},
    semantic::{MessageSymbol, ResolvedType},
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
        // Copy only validated present fields. Absent fields retain schema defaults.
        writeln!(out, "static void {name}_value_copy(const {name}_t *view, {name}_value_t *out) {{\n  memset(out, 0, sizeof(*out));").unwrap();
        if message.fields.iter().any(|field| {
            matches!(
                field.cardinality,
                Cardinality::Optional | Cardinality::Required
            ) && !matches!(field.ty, ResolvedType::Message { .. })
        }) {
            writeln!(out, "  {name}_t defaults;\n  {name}_clear(&defaults);").unwrap();
        }
        if message.fields.is_empty() {
            out.push_str("  (void)view;\n");
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
                        // A clear view has valid defaults even when not present.
                        // Arbitrary absent views may contain garbage: use a clear
                        // view for absent fields instead of following its pointers.
                        writeln!(out, "  {{\n    const {} *field = view->has_{f} ? &view->{f} : &defaults.{f};\n    out->{f}.length = field->length;\n    if (field->length != 0U) memcpy(out->{f}.data, field->data, field->length);\n  }}", c_type(&field.ty)).unwrap();
                    }
                    ResolvedType::Message { name: child, .. } => {
                        let child = type_name(child);
                        writeln!(out, "  if (view->has_{f}) {child}_value_copy(&view->{f}, &out->{f});\n  else {child}_value_clear(&out->{f});").unwrap();
                    }
                    _ => writeln!(
                        out,
                        "  out->{f} = view->has_{f} ? view->{f} : defaults.{f};"
                    )
                    .unwrap(),
                },
            }
        }
        writeln!(out, "}}\n\nvoid {name}_value_clear({name}_value_t *value) {{\n  {name}_t view;\n  if (value == NULL) return;\n  {name}_clear(&view);\n  {name}_value_copy(&view, value);\n}}\n\nwl_codec_status_t {name}_value_from_view(const {name}_t *view, {name}_value_t *out) {{\n  size_t size;\n  wl_codec_status_t status;\n  if (view == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = wlc_measure(&{name}_desc, view, &size);\n  if (status != WL_CODEC_OK) return status;\n  {name}_value_copy(view, out);\n  return WL_CODEC_OK;\n}}\n").unwrap();
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
        writeln!(out, "  return WL_CODEC_OK;\n}}\n\nwl_codec_status_t {name}_value_to_view(const {name}_value_t *value, {name}_t *out) {{\n  {name}_t view;\n  size_t size;\n  wl_codec_status_t status;\n  if (value == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  if (status == WL_CODEC_OK) status = wlc_measure(&{name}_desc, &view, &size);\n  if (status != WL_CODEC_OK) return status;\n  *out = view;\n  return WL_CODEC_OK;\n}}\n\nsize_t {name}_value_encoded_size(const {name}_value_t *value) {{\n  {name}_t view;\n  if (value == NULL || {name}_value_borrow(value, &view) != WL_CODEC_OK) return SIZE_MAX;\n  return {name}_encoded_size(&view);\n}}\n\nwl_codec_status_t {name}_value_encode(const {name}_value_t *value, uint8_t *out, size_t capacity, size_t *length) {{\n  {name}_t view;\n  wl_codec_status_t status;\n  if (value == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_value_borrow(value, &view);\n  return status == WL_CODEC_OK ? {name}_encode(&view, out, capacity, length) : status;\n}}\n\nwl_codec_status_t {name}_value_decode(const uint8_t *input, size_t length, {name}_value_t *out) {{\n  {name}_t view;\n  wl_codec_status_t status;\n  if (out == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n  status = {name}_decode(input, length, &view);\n  if (status != WL_CODEC_OK) return status;\n  {name}_value_copy(&view, out);\n  return WL_CODEC_OK;\n}}\n").unwrap();
    }
}
