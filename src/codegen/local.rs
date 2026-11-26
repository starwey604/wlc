// SPDX-License-Identifier: Apache-2.0
//! Private decoders for disposable route values. Callers discard partial output.
use super::{c_identifier, fixed, flat, names, plan::MessagePlan, type_name};
use crate::{
    ast::Cardinality,
    codec_plan::{CodecProfile, fixed_width},
    node::Node,
    semantic::{FieldSymbol, ResolvedType, SemanticModel},
};
use std::{collections::BTreeSet, fmt::Write};

pub(crate) fn name(ty: &str, profile: CodecProfile) -> String {
    if profile == CodecProfile::PositionalV1 {
        format!("{ty}_pos_decode_local")
    } else {
        format!("{ty}_decode_local")
    }
}

// Node capacity validation excludes unbounded (including transitively repeated)
// values. Nested messages always use tagged encoding, even if routed positionally.
pub(crate) fn emit_routes(s: &mut String, schema: &SemanticModel, nodes: &[Node]) {
    let mut tagged = BTreeSet::new();
    let mut positional = BTreeSet::new();
    let mut positional_profile = BTreeSet::new();
    for node in nodes {
        tagged.extend(node.calls.iter().map(|r| r.response_id));
        tagged.extend(node.serves.iter().map(|r| r.request_id));
        for r in node.send_routes.iter().chain(&node.recv_routes) {
            if r.codec == CodecProfile::PositionalV1 {
                positional_profile.insert(r.message_id);
            }
        }
        for r in &node.recv_routes {
            if r.codec == CodecProfile::PositionalV1 {
                positional.insert(r.message_id);
            } else {
                tagged.insert(r.message_id);
            }
        }
    }
    let messages = names::ordered_messages(schema).expect("validated schema");
    for m in messages.iter().rev() {
        if tagged.contains(&m.id) {
            for f in &m.fields {
                if let ResolvedType::Message { id, .. } = f.ty {
                    tagged.insert(id);
                }
            }
        }
    }
    for m in messages {
        let plan = MessagePlan::new(m);
        if tagged.contains(&m.id) && (!flat::supports(&plan) || positional_profile.contains(&m.id))
        {
            decoder(s, &plan);
        }
        if positional.contains(&m.id) {
            decoder(s, &MessagePlan::with_profile(m, CodecProfile::PositionalV1));
        }
    }
}

pub(super) fn decoder(s: &mut String, p: &MessagePlan<'_>) {
    let ty = type_name(&p.message.name);
    let func = name(&ty, p.profile);
    writeln!(s, "static wl_codec_status_t {func}(const uint8_t *input, size_t length, {ty}_t *out) {{\n  if (!out || (length && !input)) return WL_CODEC_ERR_INVALID_VALUE;\n  const uint8_t *cursor = input;\n  const uint8_t *end = length ? input + length : input;").unwrap();
    let fields = &p.message.fields;
    if fields.is_empty()
        || fields.iter().any(|f| {
            matches!(
                f.cardinality,
                Cardinality::Optional | Cardinality::Packed(_)
            )
        })
    {
        writeln!(s, "  {ty}_clear(out);").unwrap();
    }
    if p.profile == CodecProfile::PositionalV1 {
        let optional = fields
            .iter()
            .filter(|f| {
                matches!(
                    f.cardinality,
                    Cardinality::Optional | Cardinality::Packed(_)
                )
            })
            .count();
        if optional != 0 {
            let bytes = optional.div_ceil(8);
            writeln!(
                s,
                "  if (length < {bytes}U) return WL_CODEC_ERR_MALFORMED;\n  cursor += {bytes}U;"
            )
            .unwrap();
            if optional % 8 != 0 {
                writeln!(
                    s,
                    "  if (input[{}] >> {}U) return WL_CODEC_ERR_MALFORMED;",
                    bytes - 1,
                    optional % 8
                )
                .unwrap();
            }
        }
        let mut bit = 0;
        for f in fields {
            if matches!(
                f.cardinality,
                Cardinality::Optional | Cardinality::Packed(_)
            ) {
                writeln!(s, "  if (input[{}] & {}U) {{", bit / 8, 1u8 << (bit % 8)).unwrap();
                bit += 1;
            } else {
                s.push_str("  {\n");
            }
            read_field(s, f, true, fixed::supports(p));
            s.push_str("  }\n");
        }
        s.push_str("  if (cursor != end) return WL_CODEC_ERR_MALFORMED;\n");
    } else {
        if !fields.is_empty() {
            writeln!(s, "  uint64_t seen[{}] = {{0}};", fields.len().div_ceil(64)).unwrap();
        }
        s.push_str("  while (cursor != end) {\n    uint64_t key;\n    wl_codec_status_t status = wl_codec_getv(&cursor, end, &key);\n    if (status != WL_CODEC_OK) return status;\n    uint64_t number = key >> 3U;\n    uint8_t wire = (uint8_t)(key & 7U);\n    if (!number || number > UINT16_MAX || (wire != 0U && wire != 1U && wire != 2U && wire != 5U)) return WL_CODEC_ERR_MALFORMED;\n    switch ((uint16_t)number) {\n");
        for (i, (f, k)) in fields.iter().zip(&p.keys).enumerate() {
            let word = i / 64;
            let bit = 1u64 << (i % 64);
            writeln!(s, "    case {}U: {{\n      if (wire != {}U) return WL_CODEC_ERR_WIRE_TYPE;\n      if (seen[{word}] & UINT64_C({bit})) return WL_CODEC_ERR_DUPLICATE_FIELD;\n      seen[{word}] |= UINT64_C({bit});",f.number,k.wire).unwrap();
            read_field(s, f, false, false);
            s.push_str("      break;\n    }\n");
        }
        s.push_str("    default:\n      status = wl_codec_skip(&cursor, end, wire);\n      if (status != WL_CODEC_OK) return status;\n      break;\n    }\n  }\n");
        for (word, group) in fields.chunks(64).enumerate() {
            let mask = group.iter().enumerate().fold(0u64, |m, (i, f)| {
                if matches!(
                    f.cardinality,
                    Cardinality::Required | Cardinality::RequiredPacked(_)
                ) {
                    m | (1u64 << i)
                } else {
                    m
                }
            });
            if mask != 0 {
                writeln!(s, "  if ((seen[{word}] & UINT64_C({mask})) != UINT64_C({mask})) return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;").unwrap();
            }
        }
    }
    s.push_str("  return WL_CODEC_OK;\n}\n");
}

fn read_field(s: &mut String, f: &FieldSymbol, positional: bool, scalar_shape: bool) {
    let id = c_identifier(&f.name);
    let dst = format!("out->{id}");
    let get_len = "      uint64_t n;\n      wl_codec_status_t st = wl_codec_getv(&cursor, end, &n);\n      if (st != WL_CODEC_OK) return st;\n";
    if let Cardinality::Packed(count) | Cardinality::RequiredPacked(count) = f.cardinality {
        let width = fixed_width(&f.ty).unwrap();
        let bits = width * 8;
        s.push_str(get_len);
        writeln!(s, "      if (n != sizeof({dst}) || n > (size_t)(end - cursor)) return WL_CODEC_ERR_MALFORMED;\n      for (size_t i = 0; i < {count}U; ++i) {{\n        uint{bits}_t bits = wl_codec_get_le{bits}(cursor);\n        memcpy(&{dst}[i], &bits, {width}U);\n        cursor += {width}U;\n      }}").unwrap();
    } else if matches!(f.ty, ResolvedType::Bytes | ResolvedType::String) {
        s.push_str(get_len);
        writeln!(s, "      if (n > {}U) return WL_CODEC_ERR_INVALID_VALUE;\n      if (n > (size_t)(end - cursor)) return WL_CODEC_ERR_MALFORMED;",f.max_length.expect("bounded route")).unwrap();
        if f.ty == ResolvedType::String {
            s.push_str("      st = wl_codec_utf8(cursor, (size_t)n);\n      if (st != WL_CODEC_OK) return st;\n");
            writeln!(s, "      {dst}.data[n] = '\\0';").unwrap();
        }
        writeln!(s, "      wl_codec_copy_bytes({dst}.data, cursor, (size_t)n);\n      {dst}.length = (size_t)n;\n      cursor += (size_t)n;").unwrap();
    } else if let ResolvedType::Message { name, .. } = &f.ty {
        s.push_str(get_len);
        writeln!(s, "      if (n > SIZE_MAX) return WL_CODEC_ERR_OVERFLOW;\n      if (n > (size_t)(end - cursor)) return WL_CODEC_ERR_MALFORMED;\n      st = {}_decode_local(cursor, (size_t)n, &{dst});\n      if (st != WL_CODEC_OK) return st;\n      cursor += (size_t)n;",type_name(name)).unwrap();
    } else {
        fixed::emit_scalar_read(
            s,
            f,
            &dst,
            "      ",
            positional && (scalar_shape || f.ty != ResolvedType::Bool),
        );
    }
    if matches!(
        f.cardinality,
        Cardinality::Optional | Cardinality::Packed(_)
    ) {
        writeln!(s, "      out->has_{id} = true;").unwrap();
    }
}
