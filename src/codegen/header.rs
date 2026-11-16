// SPDX-License-Identifier: Apache-2.0
use std::fmt::Write;

use super::{c_identifier, c_type, type_name};
use crate::{
    ast::Cardinality,
    semantic::{MessageSymbol, ResolvedType},
};

pub(super) fn emit_message_definition(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    writeln!(output, "struct {name} {{").unwrap();
    if message.fields.is_empty() {
        writeln!(output, "  uint8_t _empty;").unwrap();
    } else {
        for field in &message.fields {
            let ident = c_identifier(&field.name);
            let ty = c_type(&field.ty);
            match field.cardinality {
                Cardinality::Optional | Cardinality::Required => {
                    if field.cardinality == Cardinality::Optional {
                        writeln!(output, "  bool has_{ident};").unwrap();
                    }
                    match (field.ty.clone(), field.max_length) {
                        (ResolvedType::String, Some(max)) => {
                            let size = usize::from(max) + 1;
                            writeln!(
                                output,
                                "  struct {{ size_t length; char data[{size}]; }} {ident};"
                            )
                            .unwrap();
                        }
                        (ResolvedType::Bytes, Some(max)) => {
                            let size = usize::from(max).max(1);
                            writeln!(
                                output,
                                "  struct {{ size_t length; uint8_t data[{size}]; }} {ident};"
                            )
                            .unwrap();
                        }
                        _ => writeln!(output, "  {ty} {ident};").unwrap(),
                    }
                }
                Cardinality::Repeated => {
                    writeln!(output, "  {ty} *{ident};").unwrap();
                    writeln!(output, "  size_t {ident}_count;").unwrap();
                    writeln!(output, "  size_t {ident}_capacity;").unwrap();
                }
                Cardinality::Packed(count) | Cardinality::RequiredPacked(count) => {
                    if matches!(field.cardinality, Cardinality::Packed(_)) {
                        writeln!(output, "  bool has_{ident};").unwrap();
                    }
                    writeln!(output, "  {ty} {ident}[{count}];").unwrap();
                }
            }
        }
    }
    writeln!(output, "}};\n").unwrap();
    for field in &message.fields {
        let Some(max) = field.max_length else {
            continue;
        };
        if !matches!(
            field.cardinality,
            Cardinality::Optional | Cardinality::Required
        ) {
            continue;
        }
        let ident = c_identifier(&field.name);
        let (arguments, helper, extra) = match field.ty {
            ResolvedType::String => ("const char *value", "str", ""),
            ResolvedType::Bytes => ("const void *value, size_t length", "bytes", ", length"),
            _ => continue,
        };
        writeln!(output, "static inline bool {name}_set_{ident}({name}_t *msg, {arguments}) {{\n  if (msg == NULL || !wl_codec_inline_{helper}_set(&msg->{ident}.length, msg->{ident}.data, {max}U, value{extra})) return false;").unwrap();
        if field.cardinality == Cardinality::Optional {
            writeln!(output, "  msg->has_{ident} = true;").unwrap();
        }
        writeln!(output, "  return true;\n}}\n").unwrap();
    }
}
