// SPDX-License-Identifier: Apache-2.0
use super::{c_identifier, c_type, type_name};
use crate::semantic::MessageSymbol;

pub(super) fn emit_message_definition(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    output.push_str(&format!("struct {name} {{\n"));
    if message.fields.is_empty() {
        output.push_str("  uint8_t _empty;\n");
    }
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let ty = c_type(&field.ty);
        match field.cardinality {
            crate::ast::Cardinality::Optional | crate::ast::Cardinality::Required => {
                output.push_str(&format!("  bool has_{field_name};\n  {ty} {field_name};\n"));
            }
            crate::ast::Cardinality::Repeated => {
                output.push_str(&format!("  {ty} *{field_name};\n  size_t {field_name}_count;\n  size_t {field_name}_capacity;\n"));
            }
            crate::ast::Cardinality::Packed(count)
            | crate::ast::Cardinality::RequiredPacked(count) => {
                output.push_str(&format!(
                    "  bool has_{field_name};\n  {ty} {field_name}[{count}];\n"
                ));
            }
        }
    }
    output.push_str("};\n");
}
