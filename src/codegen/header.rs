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
                    writeln!(output, "  bool has_{ident};").unwrap();
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
                    writeln!(output, "  bool has_{ident};").unwrap();
                    writeln!(output, "  {ty} {ident}[{count}];").unwrap();
                }
            }
        }
    }
    writeln!(output, "}};\n").unwrap();
}
