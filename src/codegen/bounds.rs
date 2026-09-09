// SPDX-License-Identifier: Apache-2.0
use super::wire::{field_wire, fixed_width};
use crate::semantic::{FieldSymbol, MessageSymbol, ResolvedType};
use std::collections::{BTreeSet, HashMap};

pub(crate) fn static_max_encoded_sizes(messages: &[&MessageSymbol]) -> HashMap<u16, Option<u64>> {
    let messages_by_id = messages
        .iter()
        .map(|message| (message.id, *message))
        .collect::<HashMap<_, _>>();
    let mut memo = HashMap::new();
    let mut visiting = BTreeSet::new();
    for message in messages {
        static_max_encoded_size(message, &messages_by_id, &mut memo, &mut visiting);
    }
    memo
}

pub(super) fn static_max_encoded_size(
    message: &MessageSymbol,
    messages: &HashMap<u16, &MessageSymbol>,
    memo: &mut HashMap<u16, Option<u64>>,
    visiting: &mut BTreeSet<u16>,
) -> Option<u64> {
    if let Some(maximum) = memo.get(&message.id) {
        return *maximum;
    }
    if !visiting.insert(message.id) {
        memo.insert(message.id, None);
        return None;
    }
    let maximum = (|| {
        let mut total = 0_u64;
        for field in &message.fields {
            let field_maximum = static_max_field_size(field, messages, memo, visiting)?;
            total = total.checked_add(field_maximum)?;
        }
        Some(total)
    })();
    visiting.remove(&message.id);
    memo.insert(message.id, maximum);
    maximum
}

pub(super) fn static_max_field_size(
    field: &FieldSymbol,
    messages: &HashMap<u16, &MessageSymbol>,
    memo: &mut HashMap<u16, Option<u64>>,
    visiting: &mut BTreeSet<u16>,
) -> Option<u64> {
    if matches!(field.cardinality, crate::ast::Cardinality::Repeated) {
        return None;
    }
    let wire = u64::from(field_wire(field));
    let tag = varint_size((u64::from(field.number) << 3) | wire);
    let body = match field.cardinality {
        crate::ast::Cardinality::Packed(count) | crate::ast::Cardinality::RequiredPacked(count) => {
            let element_size = u64::from(fixed_width(&field.ty)?);
            let payload = u64::from(count).checked_mul(element_size)?;
            varint_size(payload).checked_add(payload)?
        }
        crate::ast::Cardinality::Optional | crate::ast::Cardinality::Required => match &field.ty {
            ResolvedType::Bool => 1,
            ResolvedType::Uint8 | ResolvedType::Int8 => 2,
            ResolvedType::Uint16 | ResolvedType::Int16 => 3,
            ResolvedType::Uint32 | ResolvedType::Int32 | ResolvedType::Enum { .. } => 5,
            ResolvedType::Uint64 | ResolvedType::Int64 => 10,
            ResolvedType::Fixed32 | ResolvedType::Float32 => 4,
            ResolvedType::Fixed64 | ResolvedType::Float64 => 8,
            ResolvedType::Message { id, .. } => {
                let child = messages.get(id)?;
                let child_maximum = static_max_encoded_size(child, messages, memo, visiting)?;
                varint_size(child_maximum).checked_add(child_maximum)?
            }
            ResolvedType::Bytes | ResolvedType::String => {
                let maximum = u64::from(field.max_length?);
                varint_size(maximum).checked_add(maximum)?
            }
        },
        crate::ast::Cardinality::Repeated => return None,
    };
    tag.checked_add(body)
}

pub(super) fn varint_size(mut value: u64) -> u64 {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}
