// SPDX-License-Identifier: Apache-2.0
use std::fmt::Write;

use super::{MessageDirections, Symbols, type_name};
use crate::codec_plan::MessageCodecPlan;

pub(super) fn source(
    plan: &MessageCodecPlan<'_>,
    symbols: &Symbols,
    directions: MessageDirections,
) -> String {
    let name = symbols.message_name(plan.message);
    let ty = type_name(&plan.message.name);
    let storage = symbols.storage;
    let p = symbols.fn_prefix;
    let mut out = String::new();
    writeln!(
        out,
        "{storage}void {name}_clear({ty}_t *value) {{ if (value != NULL) {p}clear(&{name}_pos_desc, value); }}"
    )
    .unwrap();
    if directions.encode {
        writeln!(
            out,
            "{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ size_t size = 0U; return {p}positional_measure(&{name}_pos_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}"
        )
        .unwrap();
        writeln!(
            out,
            "{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length) {{ return {p}positional_encode(&{name}_pos_desc, value, out, out_capacity, out_length); }}"
        )
        .unwrap();
    }
    if directions.decode {
        writeln!(
            out,
            "{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {ty}_t *out) {{ {ty}_t scratch; return {p}positional_decode(&{name}_pos_desc, input, input_length, out, &scratch); }}"
        )
        .unwrap();
    }
    out.push('\n');
    out
}
