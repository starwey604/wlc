// SPDX-License-Identifier: Apache-2.0
//! Positional wrappers share primitive readers/writers with the tagged engine.
use super::{Symbols, type_name};
use crate::{codec_plan::MessageCodecPlan, semantic::MessageSymbol};
use std::fmt::Write;

pub(super) fn optional_count(plan: &MessageCodecPlan<'_>) -> usize {
    plan.fields
        .iter()
        .filter(|f| f.presence_bit.is_some())
        .count()
}

pub(super) fn source(plan: &MessageCodecPlan<'_>, symbols: &Symbols) -> String {
    let name = symbols.message(plan.message);
    let ty = type_name(&plan.message.name);
    let p = &symbols.helper;
    let storage = symbols.storage;
    let count = optional_count(plan);
    let mut out = String::new();
    writeln!(out, "static inline wl_codec_status_t {name}_wlc_detail_measure(const {ty}_t *value, size_t *size) {{ return {p}positional_measure(&{name}_desc, value, {count}U, size); }}\n{storage}void {name}_clear({ty}_t *value) {{ if (value != NULL) {p}clear(&{name}_desc, value); }}\n{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ size_t size; return {name}_wlc_detail_measure(value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t capacity, size_t *length) {{ return {p}positional_encode(&{name}_desc, value, {count}U, out, capacity, length); }}\n{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {ty}_t *out) {{ return {p}positional_decode(&{name}_desc, input, length, {count}U, out); }}\n").unwrap();
    out
}

pub(crate) fn owned_decode(
    message: &MessageSymbol,
    plan: &MessageCodecPlan<'_>,
    symbols: &Symbols,
) -> String {
    let name = symbols.message(message);
    let ty = type_name(&message.name);
    let p = &symbols.helper;
    let count = optional_count(plan);
    format!(
        "static inline wl_codec_status_t {name}_wlc_detail_value_decode(const uint8_t *input, size_t length, {ty}_value_t *out) {{\n  memset(out, 0, sizeof(*out));\n  return {p}positional_decode(&{name}_value_desc.wire, input, length, {count}U, out);\n}}\n"
    )
}
