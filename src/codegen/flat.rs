// SPDX-License-Identifier: Apache-2.0
//! Required, owned fields: operate on the public struct, never a layout twin.
use super::{
    MessageDirections, c_identifier, c_type, descriptor::field_descriptor_data, fixed, local,
    plan::MessagePlan, type_name,
};
use crate::{
    ast::Cardinality,
    codec_plan::{CodecProfile, ValueEncoding, field_encoding},
    semantic::ResolvedType,
};
use std::fmt::Write;

pub(super) fn supports(p: &MessagePlan<'_>) -> bool {
    p.profile == CodecProfile::SchemaV1
        && !p.message.fields.is_empty()
        && !fixed::supports(p)
        && p.message.fields.iter().all(|f| {
            f.cardinality == Cardinality::Required
                && match f.ty {
                    ResolvedType::Bytes | ResolvedType::String => f.max_length.is_some(),
                    ResolvedType::Message { .. } => false,
                    _ => true,
                }
        })
}

pub(super) fn source(s: &mut String, p: &MessagePlan<'_>, dir: MessageDirections) {
    let name = type_name(&p.message.name);
    writeln!(s, "void {name}_clear({name}_t *v) {{\n  if (!v) return;").unwrap();
    for f in &p.message.fields {
        let id = c_identifier(&f.name);
        let (_, signed, unsigned, literal, _) = field_descriptor_data(f);
        match f.ty {
            ResolvedType::Bytes => writeln!(s, "  v->{id}.length = 0U;").unwrap(),
            ResolvedType::String => {
                writeln!(s, "  v->{id}.length = {unsigned}U;").unwrap();
                if unsigned != 0 {
                    writeln!(s, "  memcpy(v->{id}.data, {literal}, {unsigned}U);").unwrap();
                }
                writeln!(s, "  v->{id}.data[{unsigned}] = '\\0';").unwrap();
            }
            _ => {
                let value = if signed == i64::MIN {
                    "INT64_MIN".into()
                } else if signed != 0 {
                    format!("INT64_C({signed})")
                } else {
                    format!("UINT64_C({unsigned})")
                };
                writeln!(s, "  v->{id} = ({}){value};", c_type(&f.ty)).unwrap();
            }
        }
    }
    s.push_str("}\n");
    if dir.encode {
        writeln!(s, "static wl_codec_status_t {name}_measure_fields(const {name}_t *v, size_t *size) {{\n  if (!v) return WL_CODEC_ERR_INVALID_VALUE;\n  uint32_t n = 0U;").unwrap();
        for (f, k) in p.message.fields.iter().zip(&p.keys) {
            let id = c_identifier(&f.name);
            let len = match field_encoding(f, p.profile) {
                ValueEncoding::Bytes | ValueEncoding::Utf8 => {
                    let max = f.max_length.unwrap();
                    writeln!(
                        s,
                        "  if (v->{id}.length > {max}U) return WL_CODEC_ERR_INVALID_VALUE;"
                    )
                    .unwrap();
                    if f.ty == ResolvedType::String {
                        writeln!(s, "  if (wl_codec_utf8((const uint8_t *)v->{id}.data, v->{id}.length) != WL_CODEC_OK) return WL_CODEC_ERR_UTF8;").unwrap();
                    }
                    format!("wl_codec_vsize(v->{id}.length) + (uint32_t)v->{id}.length")
                }
                ValueEncoding::Bool => "1U".into(),
                ValueEncoding::UnsignedVarint(_) => format!("wl_codec_vsize(v->{id})"),
                ValueEncoding::ZigzagVarint(64) => {
                    format!("wl_codec_vsize(wl_codec_zz64(v->{id}))")
                }
                ValueEncoding::ZigzagVarint(_) | ValueEncoding::EnumZigzag => {
                    format!("wl_codec_vsize(wl_codec_zz32(v->{id}))")
                }
                ValueEncoding::LittleEndian(w) | ValueEncoding::IeeeLittleEndian(w) => {
                    format!("{w}U")
                }
                ValueEncoding::Message(_) => unreachable!(),
            };
            writeln!(s, "  n += {}U + {len};", k.length).unwrap();
        }
        s.push_str("#if SIZE_MAX < UINT32_MAX\n  if (n > SIZE_MAX) return WL_CODEC_ERR_OVERFLOW;\n#endif\n  *size = (size_t)n;\n  return WL_CODEC_OK;\n}\n");
        writeln!(s, "size_t {name}_encoded_size(const {name}_t *v) {{\n  size_t n;\n  return {name}_measure_fields(v, &n) == WL_CODEC_OK ? n : SIZE_MAX;\n}}\nwl_codec_status_t {name}_encode(const {name}_t *v, uint8_t *out, size_t capacity, size_t *length) {{\n  size_t n;\n  wl_codec_status_t status = {name}_measure_fields(v, &n);\n  if (status != WL_CODEC_OK) return status;\n  if (!out || !length) return WL_CODEC_ERR_INVALID_VALUE;\n  if (capacity < n) return WL_CODEC_ERR_CAPACITY;\n  uint8_t *cursor = out;").unwrap();
        for (f, k) in p.message.fields.iter().zip(&p.keys) {
            let id = c_identifier(&f.name);
            for byte in &k.bytes[..k.length] {
                writeln!(s, "  *cursor++ = 0x{byte:02x}U;").unwrap();
            }
            if matches!(f.ty, ResolvedType::Bytes | ResolvedType::String) {
                writeln!(s, "  cursor = wl_codec_putv(cursor, v->{id}.length);\n  memcpy(cursor, v->{id}.data, v->{id}.length);\n  cursor += v->{id}.length;").unwrap();
            } else {
                fixed::emit_scalar_write(s, f, &format!("v->{id}"), "  ", false);
            }
        }
        s.push_str("  *length = (size_t)(cursor - out);\n  return WL_CODEC_OK;\n}\n");
    }
    if dir.decode {
        local::decoder(s, p);
        writeln!(s, "wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {name}_t *out) {{\n  if (!out) return WL_CODEC_ERR_INVALID_VALUE;\n  {name}_t tmp;\n  wl_codec_status_t status = {name}_decode_local(input, length, &tmp);\n  if (status != WL_CODEC_OK) return status;").unwrap();
        for f in &p.message.fields {
            let id = c_identifier(&f.name);
            if matches!(f.ty, ResolvedType::Bytes | ResolvedType::String) {
                let nul = if f.ty == ResolvedType::String {
                    " + 1U"
                } else {
                    ""
                };
                writeln!(s, "  out->{id}.length = tmp.{id}.length;\n  memcpy(out->{id}.data, tmp.{id}.data, tmp.{id}.length{nul});").unwrap();
            } else {
                writeln!(s, "  out->{id} = tmp.{id};").unwrap();
            }
        }
        s.push_str("  return WL_CODEC_OK;\n}\n");
    }
}
