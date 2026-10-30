// SPDX-License-Identifier: Apache-2.0
use super::Symbols;
use super::descriptor::emit_descriptor;
use super::features::Features;
use super::plan::MessagePlan;
use super::type_name;
use crate::semantic::MessageSymbol;
const COMMON_C: &str = include_str!("templates/common.c.in");
const EMIT_C: &str = include_str!("templates/emit.c.in");
const CODEC_TAIL_C: &str = include_str!("templates/decode.c.in");

pub(super) fn emit_source(
    messages: &[&MessageSymbol],
    codecs: &crate::codec_plan::CodecPlan<'_>,
    fingerprints: Option<&std::collections::BTreeSet<u16>>,
    symbols: &Symbols,
) -> String {
    let plans = messages
        .iter()
        .map(|message| MessagePlan::from_codec(&codecs.messages[&message.id]))
        .collect::<Vec<_>>();
    let fingerprint = |id: u16| fingerprints.is_none_or(|ids| ids.contains(&id));
    let features = Features::new(messages, codecs)
        .with_fingerprints(messages.iter().any(|m| fingerprint(m.id)));
    let mut source = symbols.render(&features.select(COMMON_C), &[]);
    if symbols.prefix.is_empty() {
        for plan in &plans {
            source.push_str(&format!(
                "#if {}_CODEC_PROFILE != {}U\n#error \"Codec source/header profile mismatch; regenerate the matched artifacts.\"\n#endif\n",
                super::upper_snake(&plan.message.name), plan.profile.code()
            ));
        }
    }
    source.push_str(&emit_fields(Sink::Bytes, &features, symbols));
    source.push_str(&symbols.render(&features.select(CODEC_TAIL_C), &[]));
    source.push_str(&symbols.render(
        &features.select(include_str!("templates/decode_fields.c.in")),
        &[
            ("DECODE", &format!("{}decode", symbols.helper)),
            ("CLEAR", &format!("{}clear(d, out)", symbols.helper)),
            ("READ_VALUE", &format!("{}read_value", symbols.helper)),
        ],
    ));
    if codecs.messages.values().any(|p| {
        p.profile == crate::codec_plan::CodecProfile::PositionalV1
            && !super::fixed::supports(&MessagePlan::from_codec(p))
    }) {
        source.push_str(&symbols.render(
            &features.select(include_str!("templates/positional.c.in")),
            &[],
        ));
    }
    if messages.iter().any(|m| fingerprint(m.id)) {
        source
            .push_str(&symbols.render(&features.select(include_str!("../fingerprint.c.in")), &[]));
        // One canonical field traversal, specialized at generation time: no sink
        // callback or per-byte mode branch on the ordinary encoder's hot path.
        source.push_str(&emit_fields(Sink::Fingerprint, &features, symbols));
    }
    let p = &symbols.helper;
    for message in messages {
        source.push_str(&format!(
            "static const {p}desc_t {}_desc;\n",
            symbols.message(message)
        ));
    }
    source.push('\n');
    let mut scalar_shapes: Vec<&MessagePlan<'_>> = Vec::new();
    for id in super::descriptor::tagged_dependencies(codecs, false) {
        let plan = MessagePlan::with_profile(codecs.messages[&id].message, codecs.profile);
        emit_descriptor(&mut source, &plan, codecs, symbols);
    }
    for plan in &plans {
        emit_descriptor(&mut source, plan, codecs, symbols);
    }
    for plan in &plans {
        let message = plan.message;
        let name = symbols.message(message);
        let ty = type_name(&message.name);
        let p = &symbols.helper;
        let storage = symbols.storage;
        if plan.profile == crate::codec_plan::CodecProfile::PositionalV1
            && !super::fixed::supports(plan)
        {
            source.push_str(&super::positional::source(
                &codecs.messages[&message.id],
                symbols,
            ));
        } else if super::fixed::supports(plan) {
            let index = scalar_shapes
                .iter()
                .position(|first| super::fixed::same_shape(first, plan))
                .unwrap_or_else(|| {
                    let index = scalar_shapes.len();
                    let needs_hash = plans.iter().any(|other| {
                        super::fixed::same_shape(plan, other) && fingerprint(other.message.id)
                    });
                    source.push_str(&super::fixed::shape_source(
                        plan, index, needs_hash, symbols,
                    ));
                    scalar_shapes.push(plan);
                    index
                });
            let shape = super::fixed::shape_name(plan, index, symbols);
            source.push_str(&super::fixed::wrappers(
                plan,
                index,
                fingerprint(message.id),
                symbols,
            ));
            source.push_str(&format!("static inline wl_codec_status_t {name}_wlc_detail_decode_fields(const uint8_t *input, size_t length, void *out) {{ return {shape}_decode(input, length, out); }}\n"));
            source.push_str(&format!("static inline size_t {name}_wlc_detail_encoded_size_fields(const void *value) {{ return {shape}_encoded_size(value); }}\nstatic inline wl_codec_status_t {name}_wlc_detail_encode_fields(const void *value, uint8_t *out, size_t capacity, size_t *length) {{ return {shape}_encode(value, out, capacity, length); }}\n"));
            continue;
        }
        if plan.profile != crate::codec_plan::CodecProfile::PositionalV1 {
            source.push_str(&format!("static inline wl_codec_status_t {name}_wlc_detail_measure(const {ty}_t *value, size_t *size) {{ return {p}measure(&{name}_desc, value, size); }}\n"));
            source.push_str(&format!("{storage}void {name}_clear({ty}_t *value) {{ if (value != NULL) {p}clear(&{name}_desc, value); }}\n"));
            if let Some(specialized) = super::packed::source(plan, symbols) {
                source.push_str(&specialized);
            } else {
                source.push_str(&format!("{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ size_t size; return {p}measure(&{name}_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n"));
                source.push_str(&format!("{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t cap, size_t *length) {{ return {p}encode(&{name}_desc, value, out, cap, length); }}\n"));
                source.push_str(&format!("{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {ty}_t *out) {{ return {p}decode(&{name}_desc, input, length, out); }}\n\n"));
            }
        }
        if fingerprint(message.id) {
            source.push_str(&format!("/* Generator-private: value must be the unmodified result of successful decode.\n * hash supplies the caller domain seed; codec has no RPC identity policy. */\n{storage}wl_codec_status_t {name}_wlc_detail_fingerprint(const {ty}_t *value, uint64_t *hash, size_t *length) {{\n  {p}hash_state_t state = {{*hash, 0U, WL_CODEC_OK}};\n  wl_codec_status_t status = {p}hash_emit_fields(&{name}_desc, value, &state);\n  if (status != WL_CODEC_OK) return status;\n  if (state.status != WL_CODEC_OK) return state.status;\n  *hash = state.hash;\n  *length = state.length;\n  return WL_CODEC_OK;\n}}\n\n"));
        }
    }
    source
}

enum Sink {
    Bytes,
    Fingerprint,
}

fn emit_fields(sink: Sink, features: &Features, symbols: &Symbols) -> String {
    let p = &symbols.helper;
    let (emit, put, copy, out, measure): (&str, &str, &str, &str, &str) = match sink {
        Sink::Bytes => (
            &format!("{p}emit"),
            &format!("{p}put"),
            &format!("{p}copy_span"),
            "uint8_t **out",
            &format!("{p}measure"),
        ),
        Sink::Fingerprint => (
            &format!("{p}hash_emit"),
            &format!("{p}hash_put"),
            &format!("{p}hash_copy_span"),
            &format!("{p}hash_state_t *out"),
            &format!("{p}measure_validated"),
        ),
    };
    symbols.render(
        &features.select(EMIT_C),
        &[
            ("EMIT", emit),
            ("PUT", put),
            ("COPY_SPAN", copy),
            ("OUT", out),
            ("MEASURE", measure),
        ],
    )
}
