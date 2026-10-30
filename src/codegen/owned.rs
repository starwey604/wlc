// SPDX-License-Identifier: Apache-2.0
//! Owned storage uses the same TLV traversal and primitive readers as views.
use super::Symbols;
use super::{c_identifier, descriptor, features::Features, fixed, plan::MessagePlan, type_name};
use crate::semantic::MessageSymbol;
use std::fmt::Write;

pub(crate) fn owned_decode_source(
    messages: &[&MessageSymbol],
    codecs: &crate::codec_plan::CodecPlan<'_>,
    symbols: &Symbols,
) -> String {
    let features = Features::owned(messages, codecs);
    let mut out = symbols.render(
        &features.select(include_str!("templates/owned_decode.c.in")),
        &[],
    );
    out.push_str(&symbols.render(
        &features.select(include_str!("templates/decode_fields.c.in")),
        &[
            ("DECODE", &format!("{}value_decode", symbols.helper)),
            ("CLEAR", &format!("{}value_clear(d, out)", symbols.helper)),
            ("READ_VALUE", &format!("{}value_read", symbols.helper)),
        ],
    ));
    out
}

pub(crate) fn owned_descriptor(
    out: &mut String,
    message: &MessageSymbol,
    codecs: &crate::codec_plan::CodecPlan<'_>,
    symbols: &Symbols,
    tagged_dependency: bool,
) {
    descriptor::emit_value_descriptor(
        out,
        &MessagePlan::from_codec(&codecs.messages[&message.id]),
        codecs,
        symbols,
    );
    if tagged_dependency {
        descriptor::emit_value_descriptor(
            out,
            &MessagePlan::with_profile(message, codecs.profile),
            codecs,
            symbols,
        );
    }
}

pub(crate) fn scalar_value_decode(
    message: &MessageSymbol,
    codecs: &crate::codec_plan::CodecPlan<'_>,
    symbols: &Symbols,
) -> Option<String> {
    let plan = MessagePlan::from_codec(&codecs.messages[&message.id]);
    if !fixed::supports(&plan) {
        return None;
    }
    let ty = type_name(&message.name);
    let name = symbols.message(message);
    let mut out = String::new();
    for field in &message.fields {
        let field = c_identifier(&field.name);
        for member in [&field, &format!("has_{field}")] {
            writeln!(out, "_Static_assert(offsetof({ty}_t, {member}) == offsetof({ty}_value_t, {member}), \"Scalar owned layout\");").unwrap();
        }
    }
    writeln!(out, "/* The shared scalar backend initializes each field through its primitive type. */\nstatic inline wl_codec_status_t {name}_wlc_detail_value_decode(const uint8_t *input, size_t length, {ty}_value_t *out) {{\n  (void){name}_value_desc;\n  return {name}_wlc_detail_decode_fields(input, length, out);\n}}\n").unwrap();
    Some(out)
}
