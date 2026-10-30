// SPDX-License-Identifier: Apache-2.0
//! Wire-format facts shared by size bounds and message planning.
pub(super) use crate::codec_plan::fixed_width;
use crate::codec_plan::{CodecProfile, field_wire_type};
use crate::semantic::FieldSymbol;

pub(super) fn field_wire(field: &FieldSymbol) -> u8 {
    field_wire_type(field, CodecProfile::SchemaV1)
}

pub(super) fn append_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
