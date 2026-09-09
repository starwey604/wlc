// SPDX-License-Identifier: Apache-2.0
//! Wire-format facts shared by size bounds and message planning.
use crate::{
    ast::Cardinality,
    semantic::{FieldSymbol, ResolvedType},
};

pub(super) fn field_wire(field: &FieldSymbol) -> u8 {
    if matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    ) {
        return 2;
    }
    match field.ty {
        ResolvedType::Fixed64 | ResolvedType::Float64 => 1,
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => 2,
        ResolvedType::Fixed32 | ResolvedType::Float32 => 5,
        _ => 0,
    }
}

pub(super) fn fixed_width(ty: &ResolvedType) -> Option<u8> {
    match ty {
        ResolvedType::Fixed32 | ResolvedType::Float32 => Some(4),
        ResolvedType::Fixed64 | ResolvedType::Float64 => Some(8),
        _ => None,
    }
}

pub(super) fn append_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
