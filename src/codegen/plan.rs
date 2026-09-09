// SPDX-License-Identifier: Apache-2.0
//! Compiler-side facts only. No new wire policy or generated runtime metadata.
use std::collections::HashMap;

use super::wire::{append_varint, field_wire, fixed_width};
use super::{CodegenError, bounds::static_max_encoded_sizes, names};
use crate::{
    ast::Cardinality,
    semantic::{FieldSymbol, MessageSymbol, SemanticModel},
};

/// Shared validation/bounds for codec and runtime generation. Constructing this
/// does not render a codec just to validate a runtime-only request.
pub(crate) struct CModel<'a> {
    pub module: String,
    pub messages: Vec<&'a MessageSymbol>,
    pub maxima: HashMap<u16, Option<u64>>,
}

impl<'a> CModel<'a> {
    pub fn new(model: &'a SemanticModel, module: &str) -> Result<Self, CodegenError> {
        let module = names::c_identifier(module);
        if module.is_empty() {
            return Err(CodegenError(
                "module name has no C identifier characters".to_owned(),
            ));
        }
        let messages = names::ordered_messages(model)?;
        let maxima = static_max_encoded_sizes(&messages);
        names::validate_names(model, &maxima)?;
        Ok(Self {
            module,
            messages,
            maxima,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Lookup {
    Linear,
    Dense,
    Binary,
}

pub(super) struct FieldKey {
    pub wire: u8,
    pub bytes: [u8; 3],
    pub length: usize,
}

impl FieldKey {
    fn new(field: &FieldSymbol) -> Self {
        let wire = field_wire(field);
        let mut value = (u64::from(field.number) << 3) | u64::from(wire);
        let mut bytes = [0; 3];
        let mut length = 0;
        loop {
            bytes[length] = (value as u8 & 127) | if value >= 128 { 128 } else { 0 };
            length += 1;
            value >>= 7;
            if value == 0 {
                break;
            }
        }
        Self {
            wire,
            bytes,
            length,
        }
    }
}

pub(super) struct PackedArray {
    pub count: u16,
    pub required: bool,
    pub width: u8,
    pub prefix: Vec<u8>,
    pub total: u64,
}

pub(super) struct MessagePlan<'a> {
    pub message: &'a MessageSymbol,
    pub lookup: Lookup,
    pub keys: Vec<FieldKey>,
    pub packed: Option<PackedArray>,
}

impl<'a> MessagePlan<'a> {
    pub fn new(message: &'a MessageSymbol) -> Self {
        let lookup = if message.fields.len() <= 8 {
            Lookup::Linear
        } else if message
            .fields
            .windows(2)
            .all(|pair| u32::from(pair[1].number) == u32::from(pair[0].number) + 1)
        {
            Lookup::Dense
        } else {
            Lookup::Binary
        };
        let keys = message.fields.iter().map(FieldKey::new).collect::<Vec<_>>();
        let packed = (|| {
            let [field] = message.fields.as_slice() else {
                return None;
            };
            let (count, required) = match field.cardinality {
                Cardinality::Packed(count) => (count, false),
                Cardinality::RequiredPacked(count) => (count, true),
                _ => return None,
            };
            let width = fixed_width(&field.ty)?;
            let payload = u64::from(count) * u64::from(width);
            let mut prefix = keys[0].bytes[..keys[0].length].to_vec();
            append_varint(payload, &mut prefix);
            let total = payload + prefix.len() as u64;
            Some(PackedArray {
                count,
                required,
                width,
                prefix,
                total,
            })
        })();
        Self {
            message,
            lookup,
            keys,
            packed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CModel, Lookup, MessagePlan};
    use crate::{analyze_schema, parse_schema, semantic::SemanticModel};

    fn schema(source: &str) -> SemanticModel {
        analyze_schema(&parse_schema(source).unwrap()).unwrap()
    }

    #[test]
    fn lookup_thresholds_use_canonical_field_order() {
        for (count, stride, expected) in [
            (0, 1, Lookup::Linear),
            (1, 1, Lookup::Linear),
            (8, 1, Lookup::Linear),
            (8, 997, Lookup::Linear),
            (9, 1, Lookup::Dense),
            (9, 997, Lookup::Binary),
            (64, 1, Lookup::Dense),
            (64, 997, Lookup::Binary),
        ] {
            let mut source = "version 1; message Sample @id(1) {".to_owned();
            for index in (0..count).rev() {
                source.push_str(&format!(
                    "optional uint32 f{index} @id({});",
                    33 + index * stride
                ));
            }
            source.push('}');
            let schema = schema(&source);
            let model = CModel::new(&schema, "sample").unwrap();
            let plan = MessagePlan::new(model.messages[0]);
            assert_eq!(plan.lookup, expected, "count={count}, stride={stride}");
            assert_eq!(plan.keys.len(), count);
            assert!(
                plan.message
                    .fields
                    .windows(2)
                    .all(|pair| pair[0].number < pair[1].number)
            );
        }
    }

    #[test]
    fn canonical_keys_cover_varint_and_wire_boundaries() {
        for (number, ty, wire, expected) in [
            (1, "uint32", 0, &[0x08][..]),
            (15, "uint32", 0, &[0x78][..]),
            (16, "uint32", 0, &[0x80, 0x01][..]),
            (2047, "uint32", 0, &[0xf8, 0x7f][..]),
            (2048, "uint32", 0, &[0x80, 0x80, 0x01][..]),
            (65535, "uint32", 0, &[0xf8, 0xff, 0x1f][..]),
            (65535, "fixed64", 1, &[0xf9, 0xff, 0x1f][..]),
            (65535, "bytes<1>", 2, &[0xfa, 0xff, 0x1f][..]),
            (65535, "fixed32", 5, &[0xfd, 0xff, 0x1f][..]),
        ] {
            let schema = schema(&format!(
                "version 1; message Sample @id(1) {{ optional {ty} value @id({number}); }}"
            ));
            let model = CModel::new(&schema, "sample").unwrap();
            let plan = MessagePlan::new(model.messages[0]);
            let key = &plan.keys[0];
            assert_eq!(key.wire, wire);
            assert_eq!(&key.bytes[..key.length], expected);
            assert!(key.bytes[key.length..].iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn packed_recipes_share_bounds_and_do_not_expand_other_shapes() {
        for (ty, width) in [
            ("fixed32", 4),
            ("float32", 4),
            ("fixed64", 8),
            ("float64", 8),
        ] {
            for count in [1, 30, 128, 65535] {
                for required in [false, true] {
                    let modifier = if required { "required " } else { "" };
                    let source = format!(
                        "version 1; message Sample @id(1) {{ {modifier}packed {ty} values[{count}] @id(65535); }}"
                    );
                    let schema = schema(&source);
                    let model = CModel::new(&schema, "sample").unwrap();
                    let plan = MessagePlan::new(model.messages[0]);
                    let recipe = plan.packed.unwrap();
                    assert_eq!(
                        (recipe.width, recipe.count, recipe.required),
                        (width, count, required)
                    );
                    assert_eq!(
                        recipe.total,
                        u64::from(width) * u64::from(count) + recipe.prefix.len() as u64
                    );
                    assert_eq!(model.maxima[&1], Some(recipe.total));
                    assert_eq!(&recipe.prefix[..3], &[0xfa, 0xff, 0x1f]);
                    if count == 65535 && width == 8 {
                        assert_eq!(recipe.prefix, [0xfa, 0xff, 0x1f, 0xf8, 0xff, 0x1f]);
                    }
                }
            }
        }
        let schema = schema(
            "version 1;
            message Empty @id(1) {}
            message Scalar @id(2) { optional fixed32 value @id(1); }
            message Mixed @id(3) { packed float32 values[30] @id(1); optional uint32 seq @id(2); }
            message Repeated @id(4) { repeated fixed32 values @id(1); }",
        );
        for message in CModel::new(&schema, "sample").unwrap().messages {
            assert!(MessagePlan::new(message).packed.is_none());
        }
    }

    #[test]
    fn model_shares_recursive_bounds_and_keeps_unbounded_values_unbounded() {
        let schema = schema(
            "version 1;
            message Parent @id(1) { optional Child child @id(1); }
            message Child @id(2) { optional string<8> name @id(1); }
            message Unbounded @id(3) { optional bytes blob @id(1); }
            message NestedUnbounded @id(4) { optional Unbounded child @id(1); }
            message Empty @id(5) {}",
        );
        let model = CModel::new(&schema, "MyCodec").unwrap();
        assert_eq!(model.module, "my_codec");
        assert_eq!(
            model
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [2, 1, 3, 4, 5]
        );
        for (id, bound) in [
            (1, Some(12)),
            (2, Some(10)),
            (3, None),
            (4, None),
            (5, Some(0)),
        ] {
            assert_eq!(model.maxima[&id], bound);
        }
    }
}
