//! Narrow specialization: a message containing exactly one fixed-width array.
//! Keep element loops bounded by the schema; never unroll arbitrary messages.
use crate::{
    ast::Cardinality,
    codegen::{c_identifier, type_name},
    semantic::{MessageSymbol, ResolvedType},
};

pub(crate) fn source(message: &MessageSymbol) -> Option<String> {
    let [field] = message.fields.as_slice() else {
        return None;
    };
    let (count, required) = match field.cardinality {
        Cardinality::Packed(n) => (n, false),
        Cardinality::RequiredPacked(n) => (n, true),
        _ => return None,
    };
    let width = match field.ty {
        ResolvedType::Fixed32 | ResolvedType::Float32 => 4,
        ResolvedType::Fixed64 | ResolvedType::Float64 => 8,
        _ => return None,
    };
    let bytes = u64::from(count) * width;
    let mut header = Vec::new();
    for mut value in [(u64::from(field.number) << 3) | 2, bytes] {
        while value >= 128 {
            header.push((value as u8) | 128);
            value >>= 7;
        }
        header.push(value as u8);
    }
    Some(
        include_str!("packed_codec.c.in")
            .replace("@NAME@", &type_name(&message.name))
            .replace("@FIELD@", &c_identifier(&field.name))
            .replace("@NUMBER@", &field.number.to_string())
            .replace("@COUNT@", &count.to_string())
            .replace("@READ_BITS@", if width == 4 {
                "uint32_t bits = ((uint32_t)in[at] << 24U) | ((uint32_t)in[at + 1U] << 16U) | ((uint32_t)in[at + 2U] << 8U) | in[at + 3U];\n      at += 4U;"
            } else {
                "uint64_t bits = 0U;\n      for (size_t b = 0U; b < 8U; ++b) bits = (bits << 8U) | in[at++];"
            })
            .replace("@BITS@", &(width * 8).to_string())
            .replace("@TOTAL@", &(bytes + header.len() as u64).to_string())
            .replace("@HEADER_LEN@", &header.len().to_string())
            .replace(
                "@HEADER@",
                &header
                    .iter()
                    .map(|v| format!("{v}U"))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .replace("@REQUIRED@", if required { "1" } else { "0" }),
    )
}
