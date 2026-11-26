//! Stable, non-cryptographic identities for diagnostics and generated metadata.
//!
//! These identities describe an exact normalized semantic model. They are not
//! wire compatibility decisions, authentication tokens, or collision-resistant
//! security hashes. Consumers should report schema and profile identities as a
//! pair when diagnosing mismatched artifacts.

use crate::{
    ast::Cardinality,
    node::Node,
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
                    hash_type(&mut hash, &field.ty, field.max_length);
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

/// Exact normalized local node identity; report alongside schema_identity.
pub fn node_identity(model: &Node) -> u64 {
    let mut hash = StableHash::new(b"wlc.node.identity.v2");
    hash.string(&model.name);
    hash.string(model.framing.name());
    hash.string(model.handlers.name());
    hash.u32(model.timeout_ms);
    hash.u8(model.rx_slots);
    for routes in [&model.send_routes, &model.recv_routes] {
        hash.len(routes.len());
        for route in routes {
            hash.string(&route.message_name);
            hash.u16(route.message_id);
            hash.string(route.codec.name());
            hash.string(route.handler.as_deref().unwrap_or(""));
        }
    }
    for routes in [&model.calls, &model.serves] {
        hash.len(routes.len());
        for rpc in routes {
            hash.string(&rpc.name);
            hash.string(&rpc.request_name);
            hash.u16(rpc.request_id);
            hash.string(&rpc.response_name);
            hash.u16(rpc.response_id);
            hash.string(rpc.handler.as_deref().unwrap_or(""));
        }
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

fn hash_type(hash: &mut StableHash, ty: &ResolvedType, max_length: Option<u16>) {
    match (ty, max_length) {
        (ResolvedType::Bytes, Some(max_length)) => {
            hash.u8(18);
            hash.u16(max_length);
            return;
        }
        (ResolvedType::String, Some(max_length)) => {
            hash.u8(19);
            hash.u16(max_length);
            return;
        }
        (_, Some(_)) => unreachable!("only string and bytes fields may carry a length bound"),
        (_, None) => {}
    }
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
