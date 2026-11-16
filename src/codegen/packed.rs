// SPDX-License-Identifier: Apache-2.0
use super::{MessageDirections, Symbols, c_identifier, plan::MessagePlan, type_name};

pub(super) fn source(
    plan: &MessagePlan<'_>,
    symbols: &Symbols,
    directions: MessageDirections,
) -> Option<String> {
    let recipe = plan.packed.as_ref()?;
    let message = plan.message;
    let field = &message.fields[0];
    let ident = c_identifier(&field.name);
    let prefix = recipe
        .prefix
        .iter()
        .map(|byte| format!("0x{byte:02x}U"))
        .collect::<Vec<_>>()
        .join(", ");
    let name = symbols.message_name(message);
    let ty = type_name(&message.name);
    let uint_type = if recipe.width == 4 {
        "uint32_t"
    } else {
        "uint64_t"
    };
    let put_le = if recipe.width == 4 {
        "wl_codec_put_le32"
    } else {
        "wl_codec_put_le64"
    };
    let get_le = if recipe.width == 4 {
        "wl_codec_get_le32"
    } else {
        "wl_codec_get_le64"
    };
    let optional_measure = if recipe.required {
        String::new()
    } else {
        format!("  if (!value->has_{ident}) return 0U;\n")
    };
    let optional_encode = if recipe.required {
        String::new()
    } else {
        format!(
            "  if (!value->has_{ident}) {{\n    if (out_length == NULL) return WL_CODEC_ERR_INVALID_VALUE;\n    *out_length = 0U;\n    return WL_CODEC_OK;\n  }}\n"
        )
    };
    let optional_presence = if recipe.required {
        String::new()
    } else {
        format!("      out->has_{ident} = true;\n")
    };

    let mut template = include_str!("../packed_codec.c.in").to_owned();
    if !directions.encode {
        template = strip_block(
            &template,
            "/* PACKED_ENCODE_BEGIN */\n",
            "/* PACKED_ENCODE_END */\n",
        );
    }
    if !directions.decode {
        template = strip_block(
            &template,
            "/* PACKED_DECODE_BEGIN */\n",
            "/* PACKED_DECODE_END */\n",
        );
    }
    let prefix_len = recipe.prefix.len().to_string();
    let count = recipe.count.to_string();
    let wire_bytes = recipe.total.to_string();
    let elem_bytes = recipe.width.to_string();
    for (token, value) in [
        ("@STORAGE@", symbols.storage),
        ("@NAME@", &name),
        ("@TYPE@", &ty),
        ("@FIELD@", &ident),
        ("@OPTIONAL_MEASURE@", &optional_measure),
        ("@OPTIONAL_ENCODE@", &optional_encode),
        ("@OPTIONAL_PRESENCE@", &optional_presence),
        ("@PREFIX_BYTES@", &prefix_len),
        ("@PREFIX_INIT@", &prefix),
        ("@COUNT@", &count),
        ("@WIRE_BYTES@", &wire_bytes),
        ("@ELEM_BYTES@", &elem_bytes),
        ("@UINT_TYPE@", uint_type),
        ("@PUT_LE@", put_le),
        ("@GET_LE@", get_le),
    ] {
        template = template.replace(token, value);
    }
    Some(template)
}

fn strip_block(source: &str, begin: &str, end: &str) -> String {
    let Some(start) = source.find(begin) else {
        return source.to_owned();
    };
    let Some(finish) = source[start..].find(end) else {
        return source.to_owned();
    };
    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..start]);
    out.push_str(&source[start + finish + end.len()..]);
    out
}
