//! Deterministic C declaration generation for schema v1.

use std::collections::{BTreeSet, HashMap};

use heck::{ToShoutySnakeCase, ToSnakeCase};
use miette::Diagnostic;
use thiserror::Error;

use crate::semantic::{
    FieldDefault, FieldSymbol, MessageSymbol, ResolvedType, SemanticModel, Symbol,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedC {
    pub header: String,
    pub source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C generation failed: {0}")]
#[diagnostic(code(wlc::codegen))]
pub struct CodegenError(pub String);

/// Emits a self-contained generated header and a source file for one schema.
/// `module_name` controls the output file include and header guard.
pub fn generate_c(model: &SemanticModel, module_name: &str) -> Result<GeneratedC, CodegenError> {
    let module = c_identifier(module_name);
    if module.is_empty() {
        return Err(CodegenError(
            "module name has no C identifier characters".to_owned(),
        ));
    }
    validate_names(model)?;
    let guard = format!("WIRELINK_GENERATED_{}_H", upper_snake(&module));
    let mut header = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <wirelink/codec.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    let messages = ordered_messages(model)?;
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!("typedef struct {name} {name}_t;\n"));
    }
    if !messages.is_empty() {
        header.push('\n');
    }
    for symbol in &model.declarations {
        if let Symbol::Enum(enumeration) = symbol {
            let name = type_name(&enumeration.name);
            header.push_str(&format!("typedef int32_t {name}_t;\n"));
            for value in &enumeration.values {
                header.push_str(&format!(
                    "#define {} INT32_C({})\n",
                    upper_snake(&value.name),
                    value.number
                ));
            }
            header.push('\n');
        }
    }
    for message in &messages {
        emit_message_definition(&mut header, message);
        header.push('\n');
    }
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!(
            "#define {}_MESSAGE_ID {}U\n",
            upper_snake(&message.name),
            message.id
        ));
        header.push_str(&format!("void {name}_clear({name}_t *value);\n"));
        header.push_str(&format!(
            "size_t {name}_encoded_size(const {name}_t *value);\n"
        ));
        header.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length);\n"));
        header.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out);\n\n"));
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    let source = emit_source(&module, &messages);
    Ok(GeneratedC { header, source })
}

fn emit_message_definition(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    output.push_str(&format!("struct {name} {{\n"));
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let ty = c_type(&field.ty);
        match field.cardinality {
            crate::ast::Cardinality::Optional => {
                output.push_str(&format!("  bool has_{field_name};\n  {ty} {field_name};\n"));
            }
            crate::ast::Cardinality::Repeated => {
                output.push_str(&format!("  {ty} *{field_name};\n  size_t {field_name}_count;\n  size_t {field_name}_capacity;\n"));
            }
        }
    }
    output.push_str("};\n");
}

fn emit_source(module: &str, messages: &[&MessageSymbol]) -> String {
    let mut source = COMMON_C.replace("@MODULE@", module);
    for message in messages {
        source.push_str(&format!(
            "static const wlc_desc_t {}_desc;\n",
            type_name(&message.name)
        ));
    }
    source.push('\n');
    for message in messages {
        emit_descriptor(&mut source, message);
    }
    for message in messages {
        let name = type_name(&message.name);
        source.push_str(&format!("void {name}_clear({name}_t *value) {{ if (value != NULL) wlc_clear(&{name}_desc, value); }}\n"));
        source.push_str(&format!("size_t {name}_encoded_size(const {name}_t *value) {{ size_t size; return wlc_measure(&{name}_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t cap, size_t *length) {{ return wlc_encode(&{name}_desc, value, out, cap, length); }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {name}_t *out) {{ return wlc_decode(&{name}_desc, input, length, out); }}\n\n"));
    }
    source
}

fn emit_descriptor(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    output.push_str(&format!("static const wlc_field_t {name}_fields[] = {{\n"));
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let (kind, signed_default, unsigned_default, string_default, nested) =
            field_descriptor_data(field);
        let (cardinality, has, count, capacity) = match field.cardinality {
            crate::ast::Cardinality::Optional => (
                "WLC_OPTIONAL",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Repeated => (
                "WLC_REPEATED",
                "0".to_owned(),
                format!("offsetof({name}_t, {field_name}_count)"),
                format!("offsetof({name}_t, {field_name}_capacity)"),
            ),
        };
        output.push_str(&format!("  {{ {}U, {cardinality}, {kind}, offsetof({name}_t, {field_name}), {has}, {count}, {capacity}, sizeof({}), {signed_default}, {unsigned_default}ULL, {string_default}, {nested} }},\n", field.number, c_type(&field.ty)));
    }
    output.push_str("};\n");
    output.push_str(&format!("static const wlc_desc_t {name}_desc = {{ {name}_fields, sizeof({name}_fields) / sizeof({name}_fields[0]) }};\n\n"));
}

fn field_descriptor_data(field: &FieldSymbol) -> (&'static str, String, String, String, String) {
    let kind = match field.ty {
        ResolvedType::Bool => "WLC_BOOL",
        ResolvedType::Bytes => "WLC_BYTES",
        ResolvedType::String => "WLC_STRING",
        ResolvedType::Int32 => "WLC_I32",
        ResolvedType::Uint32 => "WLC_U32",
        ResolvedType::Int64 => "WLC_I64",
        ResolvedType::Uint64 => "WLC_U64",
        ResolvedType::Fixed32 => "WLC_F32",
        ResolvedType::Fixed64 => "WLC_F64",
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

const COMMON_C: &str = r#"#include "@MODULE@.h"

#include <limits.h>
#include <stddef.h>
#include <string.h>

enum { WLC_OPTIONAL, WLC_REPEATED, WLC_BOOL, WLC_U32, WLC_U64, WLC_I32, WLC_I64, WLC_F32, WLC_F64, WLC_BYTES, WLC_STRING, WLC_ENUM, WLC_MESSAGE };
typedef struct wlc_desc wlc_desc_t;
typedef struct { uint16_t number; uint8_t card, kind; size_t value, has, count, capacity, element; int64_t signed_default; uint64_t unsigned_default; const char *string_default; const wlc_desc_t *nested; } wlc_field_t;
struct wlc_desc { const wlc_field_t *fields; size_t count; };

static inline wl_codec_status_t wlc_add(size_t *a, size_t b) { if (b > SIZE_MAX - *a) return WL_CODEC_ERR_OVERFLOW; *a += b; return WL_CODEC_OK; }
static inline size_t wlc_vsize(uint64_t v) { size_t n = 1U; while (v >= 128U) { v >>= 7U; ++n; } return n; }
static inline void wlc_putv(uint8_t **p, uint64_t v) { while (v >= 128U) { *(*p)++ = (uint8_t)(v | 128U); v >>= 7U; } *(*p)++ = (uint8_t)v; }
static wl_codec_status_t wlc_getv(const uint8_t *in, size_t length, size_t *at, uint64_t *out) { size_t start = *at, n = 0U; uint64_t v = 0U; while (*at < length && n < 10U) { uint8_t b = in[(*at)++]; if (n == 9U && b > 1U) return WL_CODEC_ERR_OVERFLOW; v |= (uint64_t)(b & 127U) << (7U * n++); if ((b & 128U) == 0U) { if (wlc_vsize(v) != *at - start) return WL_CODEC_ERR_MALFORMED; *out = v; return WL_CODEC_OK; } } return *at == length ? WL_CODEC_ERR_MALFORMED : WL_CODEC_ERR_OVERFLOW; }
static bool wlc_utf8(const uint8_t *s, size_t n) { size_t i = 0U; while (i < n) { uint8_t a = s[i++]; if (a < 0x80U) continue; size_t need; uint32_t v; if (a >= 0xC2U && a <= 0xDFU) { need = 1U; v = a & 0x1FU; } else if (a >= 0xE0U && a <= 0xEFU) { need = 2U; v = a & 0x0FU; } else if (a >= 0xF0U && a <= 0xF4U) { need = 3U; v = a & 0x07U; } else return false; if (need > n - i) return false; while (need-- != 0U) { uint8_t b = s[i++]; if ((b & 0xC0U) != 0x80U) return false; v = (v << 6U) | (b & 0x3FU); } if ((a == 0xE0U && v < 0x800U) || (a == 0xEDU && v >= 0xD800U) || (a == 0xF0U && v < 0x10000U) || (a == 0xF4U && v > 0x10FFFFU)) return false; } return true; }
static inline uint8_t wlc_wire(uint8_t k) { return (k == WLC_F64) ? 1U : (k == WLC_F32) ? 5U : (k == WLC_BYTES || k == WLC_STRING || k == WLC_MESSAGE) ? 2U : 0U; }
static inline uint64_t wlc_z32(int32_t v) { return ((uint32_t)v << 1U) ^ (uint32_t)-(uint32_t)(v < 0); }
static inline uint64_t wlc_z64(int64_t v) { return ((uint64_t)v << 1U) ^ (uint64_t)-(uint64_t)(v < 0); }
static inline int32_t wlc_uz32(uint32_t v) { return (int32_t)((v >> 1U) ^ (uint32_t)-(v & 1U)); }
static inline int64_t wlc_uz64(uint64_t v) { return (int64_t)((v >> 1U) ^ (uint64_t)-(v & 1U)); }
static wl_codec_status_t wlc_measure(const wlc_desc_t *, const void *, size_t *);
static void wlc_clear(const wlc_desc_t *, void *);
static wl_codec_status_t wlc_decode(const wlc_desc_t *, const uint8_t *, size_t, void *);

static wl_codec_status_t wlc_body(const wlc_field_t *f, const void *p, size_t *n) { *n = 0U; switch (f->kind) { case WLC_BOOL: { bool v = *(const bool *)p; if (v != false && v != true) return WL_CODEC_ERR_INVALID_VALUE; *n = 1U; return WL_CODEC_OK; } case WLC_U32: *n = wlc_vsize(*(const uint32_t *)p); return WL_CODEC_OK; case WLC_U64: *n = wlc_vsize(*(const uint64_t *)p); return WL_CODEC_OK; case WLC_I32: case WLC_ENUM: *n = wlc_vsize(wlc_z32(*(const int32_t *)p)); return WL_CODEC_OK; case WLC_I64: *n = wlc_vsize(wlc_z64(*(const int64_t *)p)); return WL_CODEC_OK; case WLC_F32: *n = 4U; return WL_CODEC_OK; case WLC_F64: *n = 8U; return WL_CODEC_OK; case WLC_BYTES: { const wl_codec_bytes_t *v = p; if (v->length != 0U && v->data == NULL) return WL_CODEC_ERR_INVALID_VALUE; return wlc_add(n, wlc_vsize(v->length)) == WL_CODEC_OK ? wlc_add(n, v->length) : WL_CODEC_ERR_OVERFLOW; } case WLC_STRING: { const wl_codec_string_t *v = p; if (v->length != 0U && v->data == NULL) return WL_CODEC_ERR_INVALID_VALUE; if (!wlc_utf8((const uint8_t *)v->data, v->length)) return WL_CODEC_ERR_UTF8; return wlc_add(n, wlc_vsize(v->length)) == WL_CODEC_OK ? wlc_add(n, v->length) : WL_CODEC_ERR_OVERFLOW; } default: { size_t child; wl_codec_status_t s = wlc_measure(f->nested, p, &child); if (s != WL_CODEC_OK) return s; *n = wlc_vsize(child); return wlc_add(n, child); } } }
static wl_codec_status_t wlc_measure(const wlc_desc_t *d, const void *value, size_t *out) { size_t n = 0U; if (d == NULL || value == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE; for (size_t i = 0U; i < d->count; ++i) { const wlc_field_t *f = &d->fields[i]; const uint8_t *base = value; size_t count = 1U; if (f->card == WLC_OPTIONAL) { if (!*(const bool *)(base + f->has)) continue; } else { count = *(const size_t *)(base + f->count); if ((count != 0U && *(void *const *)(base + f->value) == NULL) || count > *(const size_t *)(base + f->capacity)) return WL_CODEC_ERR_INVALID_VALUE; } for (size_t j = 0U; j < count; ++j) { size_t body; const void *p = f->card == WLC_REPEATED ? *(const uint8_t *const *)(base + f->value) + j * f->element : base + f->value; wl_codec_status_t s = wlc_body(f, p, &body); if (s != WL_CODEC_OK) return s; if ((s = wlc_add(&n, wlc_vsize(((uint64_t)f->number << 3U) | wlc_wire(f->kind)))) != WL_CODEC_OK || (s = wlc_add(&n, body)) != WL_CODEC_OK) return s; } } *out = n; return WL_CODEC_OK; }
static void wlc_clear(const wlc_desc_t *d, void *value) { uint8_t *base = value; for (size_t i = 0U; i < d->count; ++i) { const wlc_field_t *f = &d->fields[i]; void *p = base + f->value; if (f->card == WLC_REPEATED) { *(size_t *)(base + f->count) = 0U; continue; } *(bool *)(base + f->has) = false; if (f->kind == WLC_MESSAGE) { wlc_clear(f->nested, p); continue; } if (f->kind == WLC_STRING) { *(wl_codec_string_t *)p = (wl_codec_string_t){f->string_default, (size_t)f->unsigned_default}; continue; } if (f->kind == WLC_BYTES) { *(wl_codec_bytes_t *)p = (wl_codec_bytes_t){NULL, 0U}; continue; } if (f->kind == WLC_BOOL) *(bool *)p = f->unsigned_default != 0U; else if (f->kind == WLC_I32 || f->kind == WLC_ENUM) *(int32_t *)p = (int32_t)f->signed_default; else if (f->kind == WLC_I64) *(int64_t *)p = f->signed_default; else if (f->kind == WLC_U32 || f->kind == WLC_F32) *(uint32_t *)p = (uint32_t)f->unsigned_default; else if (f->kind == WLC_U64 || f->kind == WLC_F64) *(uint64_t *)p = f->unsigned_default; } }
static void wlc_put_fixed(uint8_t **p, uint64_t v, size_t n) { while (n-- != 0U) *(*p)++ = (uint8_t)(v >> (8U * n)); }
static wl_codec_status_t wlc_emit_value(const wlc_field_t *f, const void *p, uint8_t **out) { size_t child; switch (f->kind) { case WLC_BOOL: wlc_putv(out, *(const bool *)p); break; case WLC_U32: wlc_putv(out, *(const uint32_t *)p); break; case WLC_U64: wlc_putv(out, *(const uint64_t *)p); break; case WLC_I32: case WLC_ENUM: wlc_putv(out, wlc_z32(*(const int32_t *)p)); break; case WLC_I64: wlc_putv(out, wlc_z64(*(const int64_t *)p)); break; case WLC_F32: wlc_put_fixed(out, *(const uint32_t *)p, 4U); break; case WLC_F64: wlc_put_fixed(out, *(const uint64_t *)p, 8U); break; case WLC_BYTES: { const wl_codec_bytes_t *v = p; wlc_putv(out, v->length); if (v->length) { memcpy(*out, v->data, v->length); *out += v->length; } break; } case WLC_STRING: { const wl_codec_string_t *v = p; wlc_putv(out, v->length); if (v->length) { memcpy(*out, v->data, v->length); *out += v->length; } break; } default: if (wlc_measure(f->nested, p, &child) != WL_CODEC_OK) return WL_CODEC_ERR_INVALID_VALUE; wlc_putv(out, child); for (size_t i = 0U; i < f->nested->count; ++i) { const wlc_field_t *q = &f->nested->fields[i]; const uint8_t *b = p; size_t c = q->card == WLC_OPTIONAL ? (*(const bool *)(b + q->has) ? 1U : 0U) : *(const size_t *)(b + q->count); for (size_t j = 0U; j < c; ++j) { const void *x = q->card == WLC_REPEATED ? *(const uint8_t *const *)(b + q->value) + j * q->element : b + q->value; wlc_putv(out, ((uint64_t)q->number << 3U) | wlc_wire(q->kind)); if (wlc_emit_value(q, x, out) != WL_CODEC_OK) return WL_CODEC_ERR_INVALID_VALUE; } } } return WL_CODEC_OK; }
static wl_codec_status_t wlc_encode(const wlc_desc_t *d, const void *value, uint8_t *out, size_t cap, size_t *length) { size_t n; wl_codec_status_t s = wlc_measure(d, value, &n); if (s != WL_CODEC_OK || length == NULL || (n != 0U && out == NULL)) return s == WL_CODEC_OK ? WL_CODEC_ERR_INVALID_VALUE : s; if (cap < n) return WL_CODEC_ERR_CAPACITY; uint8_t *p = out; for (size_t i = 0U; i < d->count; ++i) { const wlc_field_t *f = &d->fields[i]; const uint8_t *b = value; size_t c = f->card == WLC_OPTIONAL ? (*(const bool *)(b + f->has) ? 1U : 0U) : *(const size_t *)(b + f->count); for (size_t j = 0U; j < c; ++j) { const void *x = f->card == WLC_REPEATED ? *(const uint8_t *const *)(b + f->value) + j * f->element : b + f->value; wlc_putv(&p, ((uint64_t)f->number << 3U) | wlc_wire(f->kind)); if ((s = wlc_emit_value(f, x, &p)) != WL_CODEC_OK) return s; } } *length = n; return WL_CODEC_OK; }
static wl_codec_status_t wlc_skip(uint8_t wire, const uint8_t *in, size_t n, size_t *at) { uint64_t length; wl_codec_status_t s; if (wire == 0U) return wlc_getv(in, n, at, &length); if (wire == 1U) { if (n - *at < 8U) return WL_CODEC_ERR_MALFORMED; *at += 8U; return WL_CODEC_OK; } if (wire == 5U) { if (n - *at < 4U) return WL_CODEC_ERR_MALFORMED; *at += 4U; return WL_CODEC_OK; } if (wire != 2U) return WL_CODEC_ERR_MALFORMED; if ((s = wlc_getv(in, n, at, &length)) != WL_CODEC_OK) return s; if (length > n - *at) return WL_CODEC_ERR_MALFORMED; *at += (size_t)length; return WL_CODEC_OK; }
static wl_codec_status_t wlc_read_value(const wlc_field_t *f, const uint8_t *in, size_t n, size_t *at, void *out) { uint64_t v; wl_codec_status_t s; if (f->kind == WLC_F32 || f->kind == WLC_F64) { size_t bytes = f->kind == WLC_F32 ? 4U : 8U; if (n - *at < bytes) return WL_CODEC_ERR_MALFORMED; v = 0U; while (bytes-- != 0U) v = (v << 8U) | in[(*at)++]; if (f->kind == WLC_F32) *(uint32_t *)out = (uint32_t)v; else *(uint64_t *)out = v; return WL_CODEC_OK; } if (f->kind == WLC_BYTES || f->kind == WLC_STRING || f->kind == WLC_MESSAGE) { if ((s = wlc_getv(in, n, at, &v)) != WL_CODEC_OK) return s; if (v > n - *at) return WL_CODEC_ERR_MALFORMED; size_t bytes = (size_t)v; if (f->kind == WLC_BYTES) *(wl_codec_bytes_t *)out = (wl_codec_bytes_t){in + *at, bytes}; else if (f->kind == WLC_STRING) { if (!wlc_utf8(in + *at, bytes)) return WL_CODEC_ERR_UTF8; *(wl_codec_string_t *)out = (wl_codec_string_t){(const char *)(in + *at), bytes}; } else { s = wlc_decode(f->nested, in + *at, bytes, out); if (s != WL_CODEC_OK) return s; } *at += bytes; return WL_CODEC_OK; } if ((s = wlc_getv(in, n, at, &v)) != WL_CODEC_OK) return s; if (f->kind == WLC_BOOL) { if (v > 1U) return WL_CODEC_ERR_INVALID_VALUE; *(bool *)out = v != 0U; } else if (f->kind == WLC_U32) { if (v > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW; *(uint32_t *)out = (uint32_t)v; } else if (f->kind == WLC_U64) *(uint64_t *)out = v; else if (f->kind == WLC_I32 || f->kind == WLC_ENUM) { if (v > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW; *(int32_t *)out = wlc_uz32((uint32_t)v); } else *(int64_t *)out = wlc_uz64(v); return WL_CODEC_OK; }
static wl_codec_status_t wlc_decode(const wlc_desc_t *d, const uint8_t *in, size_t n, void *out) { if (d == NULL || out == NULL || (n != 0U && in == NULL)) return WL_CODEC_ERR_INVALID_VALUE; wlc_clear(d, out); for (size_t at = 0U; at < n;) { uint64_t key; wl_codec_status_t s = wlc_getv(in, n, &at, &key); if (s != WL_CODEC_OK) return s; uint64_t number = key >> 3U; uint8_t wire = (uint8_t)(key & 7U); if (number == 0U || number > 65535U || (wire != 0U && wire != 1U && wire != 2U && wire != 5U)) return WL_CODEC_ERR_MALFORMED; const wlc_field_t *f = NULL; for (size_t i = 0U; i < d->count; ++i) if (d->fields[i].number == number) { f = &d->fields[i]; break; } if (f == NULL) { if ((s = wlc_skip(wire, in, n, &at)) != WL_CODEC_OK) return s; continue; } if (wire != wlc_wire(f->kind)) return WL_CODEC_ERR_WIRE_TYPE; uint8_t *base = out; void *p; if (f->card == WLC_OPTIONAL) { if (*(bool *)(base + f->has)) return WL_CODEC_ERR_DUPLICATE_FIELD; p = base + f->value; } else { size_t count = *(size_t *)(base + f->count); if (count >= *(size_t *)(base + f->capacity)) return WL_CODEC_ERR_CAPACITY; if (count != 0U && *(void **)(base + f->value) == NULL) return WL_CODEC_ERR_INVALID_VALUE; p = *(uint8_t **)(base + f->value) + count * f->element; } if ((s = wlc_read_value(f, in, n, &at, p)) != WL_CODEC_OK) return s; if (f->card == WLC_OPTIONAL) *(bool *)(base + f->has) = true; else ++*(size_t *)(base + f->count); } return WL_CODEC_OK; }
"#;

fn c_string(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\"\\x{byte:02X}\""))
        .collect()
}

fn c_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Bool => "bool".to_owned(),
        ResolvedType::Bytes => "wl_codec_bytes_t".to_owned(),
        ResolvedType::String => "wl_codec_string_t".to_owned(),
        ResolvedType::Int32 => "int32_t".to_owned(),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => "uint32_t".to_owned(),
        ResolvedType::Int64 => "int64_t".to_owned(),
        ResolvedType::Uint64 | ResolvedType::Fixed64 => "uint64_t".to_owned(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => {
            format!("{}_t", type_name(name))
        }
    }
}

fn ordered_messages(model: &SemanticModel) -> Result<Vec<&MessageSymbol>, CodegenError> {
    let messages: HashMap<&str, &MessageSymbol> = model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.name.as_str(), message)),
            _ => None,
        })
        .collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    fn visit<'a>(
        message: &'a MessageSymbol,
        messages: &HashMap<&'a str, &'a MessageSymbol>,
        emitted: &mut BTreeSet<&'a str>,
        ordered: &mut Vec<&'a MessageSymbol>,
    ) {
        if !emitted.insert(message.name.as_str()) {
            return;
        }
        for field in &message.fields {
            if let ResolvedType::Message { name, .. } = &field.ty {
                if let Some(child) = messages.get(name.as_str()) {
                    visit(child, messages, emitted, ordered);
                }
            }
        }
        ordered.push(message);
    }
    for symbol in &model.declarations {
        if let Symbol::Message(message) = symbol {
            visit(message, &messages, &mut emitted, &mut ordered);
        }
    }
    Ok(ordered)
}

fn validate_names(model: &SemanticModel) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for symbol in &model.declarations {
        let name = type_name(symbol.name());
        if !names.insert(name.clone()) {
            return Err(CodegenError(format!(
                "declarations collide as C identifier `{name}`"
            )));
        }
    }
    Ok(())
}

fn type_name(name: &str) -> String {
    c_identifier(name)
}
fn c_identifier(name: &str) -> String {
    let mut output: String = name
        .to_snake_case()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    if output.is_empty() {
        return output;
    }
    if output.as_bytes()[0].is_ascii_digit() {
        output.insert(0, '_');
    }
    if is_c_keyword(&output) {
        output.push('_');
    }
    output
}
fn upper_snake(name: &str) -> String {
    c_identifier(name).to_shouty_snake_case()
}

fn is_c_keyword(name: &str) -> bool {
    matches!(
        name,
        "auto"
            | "break"
            | "case"
            | "char"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extern"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "register"
            | "restrict"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "typedef"
            | "union"
            | "unsigned"
            | "void"
            | "volatile"
            | "while"
            | "_bool"
            | "_complex"
            | "_imaginary"
    )
}
