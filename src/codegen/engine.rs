// SPDX-License-Identifier: Apache-2.0
use std::collections::{BTreeSet, HashMap};

use super::{
    MessageDirections, Symbols, c_identifier, c_type, descriptor, fixed, packed, plan::MessagePlan,
    positional, type_name,
};
use crate::{
    ast::Cardinality,
    codec_plan::{CodecPlan, CodecProfile},
    semantic::{FieldDefault, MessageSymbol, ResolvedType},
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    DirectFixed(usize),
    DirectPacked,
    GenericPositional,
    GenericTagged,
}

pub(super) fn emit_source(
    module: &str,
    messages: &[&MessageSymbol],
    codecs: &CodecPlan<'_>,
    directions: &HashMap<u16, MessageDirections>,
    symbols: &Symbols,
) -> String {
    let mut source = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#include \"{module}.h\"\n#include <stddef.h>\n#include <string.h>\n\n"
    );

    let plans: Vec<MessagePlan<'_>> = messages
        .iter()
        .map(|message| MessagePlan::from_codec(&codecs.messages[&message.id]))
        .collect();

    for plan in &plans {
        source.push_str(&format!(
            "#if {}_CODEC_PROFILE != {}U\n#error \"Codec source/header profile mismatch; regenerate the matched artifacts.\"\n#endif\n",
            super::upper_snake(&plan.message.name),
            plan.profile.code()
        ));
    }
    if !plans.is_empty() {
        source.push('\n');
    }

    let mut shapes: Vec<(MessagePlan<'_>, MessageDirections)> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::with_capacity(plans.len());
    for plan in &plans {
        let dir = directions
            .get(&plan.message.id)
            .copied()
            .unwrap_or_default();
        if fixed::supports(plan) {
            let idx = if let Some(existing) = shapes
                .iter()
                .position(|(candidate, _)| fixed::same_shape(candidate, plan))
            {
                shapes[existing].1.encode |= dir.encode;
                shapes[existing].1.decode |= dir.decode;
                existing
            } else {
                let idx = shapes.len();
                shapes.push((plan.clone(), dir));
                idx
            };
            kinds.push(Kind::DirectFixed(idx));
        } else if plan.profile == CodecProfile::SchemaV1 && plan.packed.is_some() {
            kinds.push(Kind::DirectPacked);
        } else if plan.profile == CodecProfile::PositionalV1 {
            kinds.push(Kind::GenericPositional);
        } else {
            kinds.push(Kind::GenericTagged);
        }
    }

    let kind_by_id: HashMap<u16, Kind> = plans
        .iter()
        .zip(&kinds)
        .map(|(plan, kind)| (plan.message.id, *kind))
        .collect();

    let uses_generic_desc = |id: u16| -> bool {
        let dir = directions.get(&id).copied().unwrap_or_default();
        match kind_by_id[&id] {
            Kind::DirectFixed(_) | Kind::GenericPositional => false,
            Kind::DirectPacked => dir.decode,
            Kind::GenericTagged => true,
        }
    };

    let needed_desc = descriptor::tagged_dependencies(codecs, directions, &uses_generic_desc);
    let mut referenced_as_child = BTreeSet::new();
    for (plan, kind) in plans.iter().zip(&kinds) {
        let dir = directions
            .get(&plan.message.id)
            .copied()
            .unwrap_or_default();
        let emits_fields = match kind {
            Kind::GenericTagged => dir.encode || dir.decode,
            Kind::DirectPacked => dir.decode,
            Kind::DirectFixed(_) | Kind::GenericPositional => false,
        };
        if emits_fields {
            for field in &plan.message.fields {
                if let ResolvedType::Message { id, .. } = field.ty {
                    referenced_as_child.insert(id);
                }
            }
        }
    }

    let mut has_repeated_map: HashMap<u16, bool> = HashMap::new();
    for plan in &plans {
        let has_rep = plan.message.fields.iter().any(|field| {
            field.cardinality == Cardinality::Repeated
                || matches!(
                    field.ty,
                    ResolvedType::Message { id, .. } if *has_repeated_map.get(&id).unwrap_or(&false)
                )
        });
        has_repeated_map.insert(plan.message.id, has_rep);
    }

    let mut has_forward = false;
    for (plan, kind) in plans.iter().zip(&kinds) {
        let name = symbols.message_name(plan.message);
        if needed_desc.contains(&plan.message.id) {
            source.push_str(&format!("static const wl_codec_desc_t {name}_desc;\n"));
            has_forward = true;
        }
        if *kind == Kind::GenericPositional {
            source.push_str(&format!("static const wl_codec_desc_t {name}_pos_desc;\n"));
            has_forward = true;
        }
    }
    if has_forward {
        source.push('\n');
    }

    for (index, (shape, shape_dir)) in shapes.iter().enumerate() {
        let name = fixed::shape_name(index);
        source.push_str(&fixed::shape_source(shape, &name, *shape_dir, symbols));
    }

    for (plan, kind) in plans.iter().zip(&kinds) {
        let dir = directions
            .get(&plan.message.id)
            .copied()
            .unwrap_or_default();
        match kind {
            Kind::DirectFixed(shape_idx) => {
                let shape = fixed::shape_name(*shape_idx);
                source.push_str(&fixed::wrappers(plan, &shape, dir, symbols));
            }
            Kind::DirectPacked => {
                source.push_str(&packed::source(plan, symbols, dir).expect("packed plan"));
            }
            Kind::GenericPositional => {
                source.push_str(&positional::source(
                    &codecs.messages[&plan.message.id],
                    symbols,
                    dir,
                ));
                descriptor::emit_descriptor(
                    &mut source,
                    plan,
                    symbols,
                    true,
                    false,
                    dir,
                    false,
                    true,
                );
            }
            Kind::GenericTagged => {
                let name = symbols.message_name(plan.message);
                let ty = type_name(&plan.message.name);
                let storage = symbols.storage;
                let p = symbols.fn_prefix;
                if needed_desc.contains(&plan.message.id) {
                    source.push_str(&format!(
                        "{storage}void {name}_clear({ty}_t *value) {{ {p}clear(&{name}_desc, value); }}\n"
                    ));
                } else {
                    source.push_str(&emit_direct_clear(plan, symbols));
                }
                if dir.encode {
                    source.push_str(&format!(
                        "{storage}size_t {name}_encoded_size(const {ty}_t *value) {{ size_t size = 0U; return {p}measure(&{name}_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n{storage}wl_codec_status_t {name}_encode(const {ty}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length) {{ return {p}encode(&{name}_desc, value, out, out_capacity, out_length); }}\n"
                    ));
                }
                if dir.decode {
                    source.push_str(&format!(
                        "{storage}wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {ty}_t *out) {{ if (out == NULL) return WL_CODEC_ERR_INVALID_VALUE; {ty}_t scratch; return {p}decode(&{name}_desc, input, input_length, out, &scratch); }}\n"
                    ));
                }
                source.push('\n');
            }
        }

        if needed_desc.contains(&plan.message.id) {
            let emit_fields = match kind {
                Kind::GenericTagged => true,
                Kind::GenericPositional => true,
                Kind::DirectPacked => dir.decode || plan.profile == CodecProfile::PositionalV1,
                Kind::DirectFixed(_) => plan.profile == CodecProfile::PositionalV1,
            };
            let is_child = referenced_as_child.contains(&plan.message.id);
            descriptor::emit_descriptor(
                &mut source,
                plan,
                symbols,
                emit_fields,
                is_child,
                dir,
                has_repeated_map[&plan.message.id],
                false,
            );
        }
    }

    source
}

fn emit_direct_clear(plan: &MessagePlan<'_>, symbols: &Symbols) -> String {
    let name = symbols.message_name(plan.message);
    let ty = type_name(&plan.message.name);
    let storage = symbols.storage;
    let mut out = format!(
        "{storage}void {name}_clear({ty}_t *value) {{\n  if (value == NULL) return;\n  memset(value, 0, sizeof(*value));\n"
    );
    for field in &plan.message.fields {
        if !matches!(
            field.cardinality,
            Cardinality::Optional | Cardinality::Required
        ) {
            continue;
        }
        let ident = c_identifier(&field.name);
        if let ResolvedType::Message { name: child, .. } = &field.ty {
            let child_fn = type_name(child);
            out.push_str(&format!("  {child_fn}_clear(&value->{ident});\n"));
            continue;
        }
        let Some(default) = &field.default else {
            continue;
        };
        let (_, signed, unsigned, literal, _) = descriptor::field_descriptor_data(field);
        match default {
            FieldDefault::Bool(true) => {
                out.push_str(&format!("  value->{ident} = true;\n"));
            }
            FieldDefault::String(s) if !s.is_empty() => {
                if field.max_length.is_some() {
                    out.push_str(&format!(
                        "  value->{ident}.length = {unsigned}U;\n  memcpy(value->{ident}.data, {literal}, {unsigned}U);\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "  value->{ident}.data = {literal};\n  value->{ident}.length = {unsigned}U;\n"
                    ));
                }
            }
            FieldDefault::Int8(_)
            | FieldDefault::Int16(_)
            | FieldDefault::Int32(_)
            | FieldDefault::Int64(_)
            | FieldDefault::Enum(_)
                if signed != 0 =>
            {
                out.push_str(&format!(
                    "  value->{ident} = ({})({signed}LL);\n",
                    c_type(&field.ty)
                ));
            }
            _ if unsigned != 0 => {
                out.push_str(&format!(
                    "  value->{ident} = ({})UINT64_C({unsigned});\n",
                    c_type(&field.ty)
                ));
            }
            _ => {}
        }
    }
    out.push_str("}\n");
    out
}
