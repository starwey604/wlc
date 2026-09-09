//! Narrow specialization: a message containing exactly one fixed-width array.
//! Keep element loops bounded by the schema; never unroll arbitrary messages.
use super::{c_identifier, plan::MessagePlan, type_name};
use crate::template::render;

pub(super) fn source(plan: &MessagePlan<'_>) -> Option<String> {
    let recipe = plan.packed.as_ref()?;
    let message = plan.message;
    let field = &message.fields[0];
    Some(render(
        include_str!("../packed_codec.c.in"),
        &[
            ("NAME", &type_name(&message.name)),
            ("FIELD", &c_identifier(&field.name)),
            ("NUMBER", &field.number.to_string()),
            ("COUNT", &recipe.count.to_string()),
            (
                "READ_BITS",
                if recipe.width == 4 {
                    "uint32_t bits = ((uint32_t)in[at] << 24U) | ((uint32_t)in[at + 1U] << 16U) | ((uint32_t)in[at + 2U] << 8U) | in[at + 3U];\n      at += 4U;"
                } else {
                    "uint64_t bits = 0U;\n      for (size_t b = 0U; b < 8U; ++b) bits = (bits << 8U) | in[at++];"
                },
            ),
            ("BITS", &(recipe.width * 8).to_string()),
            ("TOTAL", &recipe.total.to_string()),
            ("HEADER_LEN", &recipe.prefix.len().to_string()),
            (
                "HEADER",
                &recipe
                    .prefix
                    .iter()
                    .map(|v| format!("{v}U"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("REQUIRED", if recipe.required { "1" } else { "0" }),
        ],
    ))
}
