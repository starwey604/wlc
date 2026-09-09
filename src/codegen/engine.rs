// SPDX-License-Identifier: Apache-2.0
use super::descriptor::emit_descriptor;
use super::plan::MessagePlan;
use super::type_name;
use crate::semantic::MessageSymbol;
use crate::template::render;
const COMMON_C: &str = include_str!("templates/common.c.in");
const EMIT_C: &str = include_str!("templates/emit.c.in");
const CODEC_TAIL_C: &str = include_str!("templates/decode.c.in");

pub(super) fn emit_source(module: &str, messages: &[&MessageSymbol]) -> String {
    let plans = messages
        .iter()
        .map(|message| MessagePlan::new(message))
        .collect::<Vec<_>>();
    let mut source = render(COMMON_C, &[("MODULE", module)]);
    source.push_str(&emit_fields(Sink::Bytes));
    source.push_str(CODEC_TAIL_C);
    source.push_str(include_str!("../fingerprint.c.in"));
    // One canonical field traversal, specialized at generation time: no sink
    // callback or per-byte mode branch on the ordinary encoder's hot path.
    source.push_str(&emit_fields(Sink::Fingerprint));
    for message in messages {
        source.push_str(&format!(
            "static const wlc_desc_t {}_desc;\n",
            type_name(&message.name)
        ));
    }
    source.push('\n');
    for plan in &plans {
        emit_descriptor(&mut source, plan);
    }
    for plan in &plans {
        let message = plan.message;
        let name = type_name(&message.name);
        source.push_str(&format!("void {name}_clear({name}_t *value) {{ if (value != NULL) wlc_clear(&{name}_desc, value); }}\n"));
        if let Some(specialized) = super::packed::source(plan) {
            source.push_str(&specialized);
        } else {
            source.push_str(&format!("size_t {name}_encoded_size(const {name}_t *value) {{ size_t size; return wlc_measure(&{name}_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n"));
            source.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t cap, size_t *length) {{ return wlc_encode(&{name}_desc, value, out, cap, length); }}\n"));
            source.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {name}_t *out) {{ return wlc_decode(&{name}_desc, input, length, out); }}\n\n"));
        }
        source.push_str(&format!("/* Generator-private: value must be the unmodified result of successful decode.\n * hash supplies the caller domain seed; codec has no RPC identity policy. */\nwl_codec_status_t {name}_wlc_detail_fingerprint(const {name}_t *value, uint64_t *hash, size_t *length) {{\n  wlc_hash_state_t state = {{*hash, 0U, WL_CODEC_OK}};\n  wl_codec_status_t status = wlc_hash_emit_fields(&{name}_desc, value, &state);\n  if (status != WL_CODEC_OK) return status;\n  if (state.status != WL_CODEC_OK) return state.status;\n  *hash = state.hash;\n  *length = state.length;\n  return WL_CODEC_OK;\n}}\n\n"));
    }
    source
}

enum Sink {
    Bytes,
    Fingerprint,
}

fn emit_fields(sink: Sink) -> String {
    let (emit, put, copy, out, measure) = match sink {
        Sink::Bytes => (
            "wlc_emit",
            "wlc_put",
            "wlc_copy_span",
            "uint8_t **out",
            "wlc_measure",
        ),
        Sink::Fingerprint => (
            "wlc_hash_emit",
            "wlc_hash_put",
            "wlc_hash_copy_span",
            "wlc_hash_state_t *out",
            "wlc_measure_validated",
        ),
    };
    render(
        EMIT_C,
        &[
            ("EMIT", emit),
            ("PUT", put),
            ("COPY_SPAN", copy),
            ("OUT", out),
            ("MEASURE", measure),
        ],
    )
}
