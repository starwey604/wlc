// SPDX-License-Identifier: Apache-2.0
//! Wire encoding and owned-storage capabilities, independent of RPC guarantees.
use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    ast::Cardinality,
    semantic::{FieldSymbol, MessageSymbol, ResolvedType, SemanticModel, Symbol},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CodecProfile {
    SchemaV1,
    ManagedTagged,
    PositionalV1,
}

impl CodecProfile {
    /// Generated C contract discriminator; never transmitted as a format tag.
    pub fn code(self) -> u8 {
        match self {
            Self::SchemaV1 => 1,
            Self::ManagedTagged => 2,
            Self::PositionalV1 => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::SchemaV1 => "schema-v1",
            Self::ManagedTagged => "managed-rpc-v4-tagged-varuint32",
            Self::PositionalV1 => "positional-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueEncoding {
    Bool,
    UnsignedVarint(u8),
    ZigzagVarint(u8),
    EnumZigzag,
    BigEndian(u8),
    IeeeBigEndian(u8),
    Bytes,
    Utf8,
    Message(u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldPlan<'a> {
    pub field: &'a FieldSymbol,
    pub encoding: ValueEncoding,
    /// Tagged wire type; absent in positional-v1.
    pub wire_type: Option<u8>,
    /// Optional scalar/packed fields receive bits in ascending field-ID order.
    pub presence_bit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageCodecPlan<'a> {
    pub message: &'a MessageSymbol,
    pub profile: CodecProfile,
    pub fields: Vec<FieldPlan<'a>>,
    pub presence_bytes: usize,
    /// None means unbounded. Overflow is an error, never an unbounded fallback.
    pub max_encoded_size: Option<u64>,
}

impl MessageCodecPlan<'_> {
    pub fn has_owned_value(&self) -> bool {
        self.max_encoded_size.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("codec plan failed: {0}")]
pub struct CodecPlanError(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecPlan<'a> {
    pub profile: CodecProfile,
    pub messages: BTreeMap<u16, MessageCodecPlan<'a>>,
}

impl<'a> CodecPlan<'a> {
    pub fn new(schema: &'a SemanticModel, profile: CodecProfile) -> Result<Self, CodecPlanError> {
        let ids = schema
            .declarations
            .iter()
            .filter_map(|symbol| match symbol {
                Symbol::Message(message) => Some(message.id),
                Symbol::Enum(_) => None,
            });
        Self::selected(schema, profile, ids)
    }

    /// Plan selected messages and their nested dependencies, not unrelated routes.
    pub fn selected(
        schema: &'a SemanticModel,
        profile: CodecProfile,
        ids: impl IntoIterator<Item = u16>,
    ) -> Result<Self, CodecPlanError> {
        let messages = schema
            .declarations
            .iter()
            .filter_map(|symbol| match symbol {
                Symbol::Message(message) => Some((message.id, message)),
                Symbol::Enum(_) => None,
            })
            .collect::<BTreeMap<_, _>>();
        let mut plans = BTreeMap::new();
        for id in ids {
            build_message(id, profile, &messages, &mut plans, &mut BTreeSet::new())?;
        }
        Ok(Self {
            profile,
            messages: plans,
        })
    }
}

pub(crate) fn field_encoding(field: &FieldSymbol, profile: CodecProfile) -> ValueEncoding {
    use ValueEncoding::*;
    let scalar = !matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    );
    match field.ty {
        ResolvedType::Bool => Bool,
        ResolvedType::Uint8 => UnsignedVarint(8),
        ResolvedType::Uint16 => UnsignedVarint(16),
        ResolvedType::Uint32 => UnsignedVarint(32),
        ResolvedType::Uint64 => UnsignedVarint(64),
        ResolvedType::Int8 => ZigzagVarint(8),
        ResolvedType::Int16 => ZigzagVarint(16),
        ResolvedType::Int32 => ZigzagVarint(32),
        ResolvedType::Int64 => ZigzagVarint(64),
        ResolvedType::Fixed32 if scalar && profile != CodecProfile::SchemaV1 => UnsignedVarint(32),
        ResolvedType::Fixed32 => BigEndian(4),
        ResolvedType::Fixed64 => BigEndian(8),
        ResolvedType::Float32 => IeeeBigEndian(4),
        ResolvedType::Float64 => IeeeBigEndian(8),
        ResolvedType::Enum { .. } => EnumZigzag,
        ResolvedType::Bytes => Bytes,
        ResolvedType::String => Utf8,
        ResolvedType::Message { id, .. } => Message(id),
    }
}

pub(crate) fn field_wire_type(field: &FieldSymbol, profile: CodecProfile) -> u8 {
    if matches!(
        field.cardinality,
        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
    ) {
        return 2;
    }
    match field_encoding(field, profile) {
        ValueEncoding::BigEndian(8) | ValueEncoding::IeeeBigEndian(8) => 1,
        ValueEncoding::Bytes | ValueEncoding::Utf8 | ValueEncoding::Message(_) => 2,
        ValueEncoding::BigEndian(4) | ValueEncoding::IeeeBigEndian(4) => 5,
        _ => 0,
    }
}

pub(crate) fn fixed_width(ty: &ResolvedType) -> Option<u8> {
    match ty {
        ResolvedType::Fixed32 | ResolvedType::Float32 => Some(4),
        ResolvedType::Fixed64 | ResolvedType::Float64 => Some(8),
        _ => None,
    }
}

fn build_message<'a>(
    id: u16,
    profile: CodecProfile,
    messages: &BTreeMap<u16, &'a MessageSymbol>,
    plans: &mut BTreeMap<u16, MessageCodecPlan<'a>>,
    visiting: &mut BTreeSet<u16>,
) -> Result<(), CodecPlanError> {
    if plans.contains_key(&id) {
        return Ok(());
    }
    let message = *messages
        .get(&id)
        .ok_or_else(|| CodecPlanError(format!("unknown message ID {id}")))?;
    if !visiting.insert(id) {
        return Err(CodecPlanError(format!(
            "recursive message `{}`",
            message.name
        )));
    }
    let mut fields = Vec::new();
    let mut ordered = message.fields.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|field| field.number);
    let mut optional_count: usize = 0;
    let mut maximum = Some(0_u64);
    for field in ordered {
        let encoding = field_encoding(field, profile);
        if profile == CodecProfile::PositionalV1
            && (field.cardinality == Cardinality::Repeated
                || matches!(
                    encoding,
                    ValueEncoding::Bytes | ValueEncoding::Utf8 | ValueEncoding::Message(_)
                ))
        {
            return Err(CodecPlanError(format!(
                "positional-v1 does not support `{}.{}` (nested/repeated/bytes/string); select a tagged codec",
                message.name, field.name
            )));
        }
        let mut body = match encoding {
            ValueEncoding::Bool => Some(1),
            ValueEncoding::UnsignedVarint(bits) | ValueEncoding::ZigzagVarint(bits) => {
                Some(u64::from(bits).div_ceil(7))
            }
            ValueEncoding::EnumZigzag => Some(5),
            ValueEncoding::BigEndian(width) | ValueEncoding::IeeeBigEndian(width) => {
                Some(u64::from(width))
            }
            ValueEncoding::Bytes | ValueEncoding::Utf8 => field.max_length.map(u64::from),
            ValueEncoding::Message(child) => {
                build_message(child, profile, messages, plans, visiting)?;
                plans[&child].max_encoded_size
            }
        };
        if matches!(
            encoding,
            ValueEncoding::Bytes | ValueEncoding::Utf8 | ValueEncoding::Message(_)
        ) {
            body = with_length(body, message, field)?;
        }
        match field.cardinality {
            Cardinality::Repeated => body = None,
            Cardinality::Packed(count) | Cardinality::RequiredPacked(count) => {
                let width = fixed_width(&field.ty).ok_or_else(|| {
                    CodecPlanError(format!(
                        "unsupported packed field `{}.{}`",
                        message.name, field.name
                    ))
                })?;
                let bytes = u64::from(count)
                    .checked_mul(u64::from(width))
                    .ok_or_else(|| overflow(message, field))?;
                body = with_length(Some(bytes), message, field)?;
            }
            _ => {}
        }
        let wire_type =
            (profile != CodecProfile::PositionalV1).then(|| field_wire_type(field, profile));
        if let Some(wire) = wire_type {
            body = body
                .map(|n| {
                    n.checked_add(varint_size(
                        (u64::from(field.number) << 3) | u64::from(wire),
                    ))
                    .ok_or_else(|| overflow(message, field))
                })
                .transpose()?;
        }
        maximum = match (maximum, body) {
            (Some(total), Some(body)) => Some(
                total
                    .checked_add(body)
                    .ok_or_else(|| overflow(message, field))?,
            ),
            _ => None,
        };
        let presence_bit = if profile == CodecProfile::PositionalV1
            && matches!(
                field.cardinality,
                Cardinality::Optional | Cardinality::Packed(_)
            ) {
            let bit = optional_count;
            optional_count += 1;
            Some(bit)
        } else {
            None
        };
        fields.push(FieldPlan {
            field,
            encoding,
            wire_type,
            presence_bit,
        });
    }
    let presence_bytes = optional_count.div_ceil(8);
    if let Some(size) = &mut maximum {
        *size = size
            .checked_add(presence_bytes as u64)
            .ok_or_else(|| CodecPlanError(format!("wire size overflow in `{}`", message.name)))?;
    }
    visiting.remove(&id);
    plans.insert(
        id,
        MessageCodecPlan {
            message,
            profile,
            fields,
            presence_bytes,
            max_encoded_size: maximum,
        },
    );
    Ok(())
}

fn with_length(
    size: Option<u64>,
    message: &MessageSymbol,
    field: &FieldSymbol,
) -> Result<Option<u64>, CodecPlanError> {
    size.map(|n| {
        n.checked_add(varint_size(n))
            .ok_or_else(|| overflow(message, field))
    })
    .transpose()
}

fn overflow(message: &MessageSymbol, field: &FieldSymbol) -> CodecPlanError {
    CodecPlanError(format!(
        "wire size overflow at `{}.{}`",
        message.name, field.name
    ))
}

fn varint_size(mut value: u64) -> u64 {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}
