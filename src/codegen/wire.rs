// SPDX-License-Identifier: Apache-2.0
use crate::semantic::ResolvedType;

pub(crate) fn wire_type(ty: &ResolvedType, is_packed: bool) -> u8 {
    if is_packed {
        return 2;
    }
    match ty {
        ResolvedType::Fixed64 | ResolvedType::Float64 => 1,
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => 2,
        ResolvedType::Fixed32 | ResolvedType::Float32 => 5,
        _ => 0,
    }
}
