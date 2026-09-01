//! Stable, non-cryptographic identities for diagnostics and generated metadata.
//!
//! These identities describe an exact normalized semantic model. They are not
//! wire compatibility decisions, authentication tokens, or collision-resistant
//! security hashes. Consumers should report schema and profile identities as a
//! pair when diagnosing mismatched artifacts.

use crate::{
    ast::Cardinality,
    profile_semantic::{BindingProfileModel, DeliveryPolicy, RetainedRouteKind, RpcStatusDomain},
    semantic::{FieldDefault, ResolvedType, SemanticModel, Symbol},
};

/// Name of the stable diagnostic identity algorithm.
pub const IDENTITY_ALGORITHM: &str = "fnv1a64-v1";

const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Return an exact identity for a normalized schema semantic model.
///
/// Source whitespace and declaration order do not affect the result. Schema
/// revision, names, IDs, reservations, field wire identity, defaults, and enum
/// values do. This is deliberately stricter than wire compatibility.
pub fn schema_identity(model: &SemanticModel) -> u64 {
    let mut hash = StableHash::new(b"wlc.schema.identity.v1");
    hash.u32(model.version);
    hash.len(model.reserved_ids.len());
    for id in &model.reserved_ids {
        hash.u16(*id);
    }
    hash.len(model.declarations.len());
    for symbol in &model.declarations {
        match symbol {
            Symbol::Message(message) => {
                hash.u8(1);
                hash.string(&message.name);
                hash.u16(message.id);
                hash.len(message.reserved_numbers.len());
                for number in &message.reserved_numbers {
                    hash.u16(*number);
                }
                hash.len(message.fields.len());
                for field in &message.fields {
                    hash.string(&field.name);
                    hash.u16(field.number);
                    hash_cardinality(&mut hash, field.cardinality);
                    hash_type(&mut hash, &field.ty);
                    hash_default(&mut hash, field.default.as_ref());
                }
            }
            Symbol::Enum(enumeration) => {
                hash.u8(2);
                hash.string(&enumeration.name);
                hash.u16(enumeration.id);
                hash.len(enumeration.reserved_numbers.len());
                for number in &enumeration.reserved_numbers {
                    hash.i32(*number);
                }
                hash.len(enumeration.values.len());
                for value in &enumeration.values {
                    hash.string(&value.name);
                    hash.i32(value.number);
                }
            }
        }
    }
    hash.finish()
}

/// Return an exact identity for a resolved application binding profile.
///
/// The profile identity covers resolved names, IDs, field numbers, service
/// roles, and delivery policy. It intentionally does not include the complete
/// schema; report it together with [`schema_identity`] when identifying a
/// generated application contract.
pub fn binding_profile_identity(model: &BindingProfileModel) -> u64 {
    let mut hash = StableHash::new(b"wlc.binding-profile.identity.v1");
    hash.u32(model.version);
    hash.len(model.retained_routes.len());
    for route in &model.retained_routes {
        hash.u8(match route.kind {
            RetainedRouteKind::Latest => 1,
            RetainedRouteKind::Fifo => 2,
        });
        hash.string(&route.message_name);
        hash.u16(route.message_id);
        hash_delivery(&mut hash, route.delivery);
    }
    hash.len(model.rpc_services.len());
    for service in &model.rpc_services {
        hash.string(&service.name);
        hash.string(&service.request_name);
        hash.u16(service.request_id);
        hash.string(&service.response_name);
        hash.u16(service.response_id);
        hash.string(&service.request_operation_id.name);
        hash.u16(service.request_operation_id.number);
        hash.string(&service.response_operation_id.name);
        hash.u16(service.response_operation_id.number);
        hash.string(&service.response_status.name);
        hash.u16(service.response_status.number);
        match &service.status_domain {
            RpcStatusDomain::Int32 => hash.u8(1),
            RpcStatusDomain::Enum { name, id } => {
                hash.u8(2);
                hash.string(name);
                hash.u16(*id);
            }
        }
        hash_delivery(&mut hash, service.request_delivery);
        hash_delivery(&mut hash, service.response_delivery);
    }
    hash.finish()
}

fn hash_cardinality(hash: &mut StableHash, cardinality: Cardinality) {
    match cardinality {
        Cardinality::Optional => hash.u8(1),
        Cardinality::Repeated => hash.u8(2),
        Cardinality::Packed(count) => {
            hash.u8(3);
            hash.u16(count);
        }
        Cardinality::Required => hash.u8(4),
        Cardinality::RequiredPacked(count) => {
            hash.u8(5);
            hash.u16(count);
        }
    }
}

fn hash_type(hash: &mut StableHash, ty: &ResolvedType) {
    match ty {
        ResolvedType::Bool => hash.u8(1),
        ResolvedType::Bytes => hash.u8(2),
        ResolvedType::String => hash.u8(3),
        ResolvedType::Int32 => hash.u8(4),
        ResolvedType::Uint32 => hash.u8(5),
        ResolvedType::Int64 => hash.u8(6),
        ResolvedType::Uint64 => hash.u8(7),
        ResolvedType::Fixed32 => hash.u8(8),
        ResolvedType::Fixed64 => hash.u8(9),
        ResolvedType::Float32 => hash.u8(10),
        ResolvedType::Float64 => hash.u8(11),
        ResolvedType::Message { id, name } => {
            hash.u8(12);
            hash.u16(*id);
            hash.string(name);
        }
        ResolvedType::Enum { id, name } => {
            hash.u8(13);
            hash.u16(*id);
            hash.string(name);
        }
        ResolvedType::Int8 => hash.u8(14),
        ResolvedType::Uint8 => hash.u8(15),
        ResolvedType::Int16 => hash.u8(16),
        ResolvedType::Uint16 => hash.u8(17),
    }
}

fn hash_default(hash: &mut StableHash, default: Option<&FieldDefault>) {
    let Some(default) = default else {
        hash.u8(0);
        return;
    };
    match default {
        FieldDefault::Bool(value) => {
            hash.u8(1);
            hash.u8(u8::from(*value));
        }
        FieldDefault::String(value) => {
            hash.u8(2);
            hash.string(value);
        }
        FieldDefault::Int32(value) => {
            hash.u8(3);
            hash.i32(*value);
        }
        FieldDefault::Uint32(value) => {
            hash.u8(4);
            hash.u32(*value);
        }
        FieldDefault::Int64(value) => {
            hash.u8(5);
            hash.i64(*value);
        }
        FieldDefault::Uint64(value) => {
            hash.u8(6);
            hash.u64(*value);
        }
        FieldDefault::Fixed32(value) => {
            hash.u8(7);
            hash.u32(*value);
        }
        FieldDefault::Fixed64(value) => {
            hash.u8(8);
            hash.u64(*value);
        }
        FieldDefault::Enum(value) => {
            hash.u8(9);
            hash.i32(*value);
        }
        FieldDefault::Int8(value) => {
            hash.u8(10);
            hash.i32(i32::from(*value));
        }
        FieldDefault::Uint8(value) => {
            hash.u8(11);
            hash.u32(u32::from(*value));
        }
        FieldDefault::Int16(value) => {
            hash.u8(12);
            hash.i32(i32::from(*value));
        }
        FieldDefault::Uint16(value) => {
            hash.u8(13);
            hash.u32(u32::from(*value));
        }
    }
}

fn hash_delivery(hash: &mut StableHash, delivery: DeliveryPolicy) {
    hash.u8(match delivery {
        DeliveryPolicy::Unreliable => 1,
        DeliveryPolicy::Reliable => 2,
    });
}

struct StableHash(u64);

impl StableHash {
    fn new(domain: &[u8]) -> Self {
        let mut hash = Self(FNV1A_OFFSET);
        hash.bytes(domain);
        hash
    }

    fn finish(self) -> u64 {
        self.0
    }

    fn raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV1A_PRIME);
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        self.raw(bytes);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn len(&mut self, value: usize) {
        self.u64(value as u64);
    }

    fn u8(&mut self, value: u8) {
        self.raw(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.raw(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.raw(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.raw(&value.to_be_bytes());
    }
}
