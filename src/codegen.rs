//! Deterministic C declaration generation for schema v1.

use std::collections::{BTreeSet, HashMap};

use heck::{ToShoutySnakeCase, ToSnakeCase};
use miette::Diagnostic;
use thiserror::Error;

use crate::semantic::{
    FieldDefault, FieldSymbol, MessageSymbol, ResolvedType, SemanticModel, Symbol,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedC {
    pub header: String,
    pub source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C generation failed: {0}")]
#[diagnostic(code(wlc::codegen))]
pub struct CodegenError(pub String);

/// Emits a self-contained generated header and a source file for one schema.
/// `module_name` controls the output file include and header guard.
pub fn generate_c(model: &SemanticModel, module_name: &str) -> Result<GeneratedC, CodegenError> {
    let module = c_identifier(module_name);
    if module.is_empty() {
        return Err(CodegenError(
            "module name has no C identifier characters".to_owned(),
        ));
    }
    validate_names(model)?;
    let guard = format!("WIRELINK_GENERATED_{}_H", upper_snake(&module));
    let mut header = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <wirelink/codec.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    let messages = ordered_messages(model)?;
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!("typedef struct {name} {name}_t;\n"));
    }
    if !messages.is_empty() {
        header.push('\n');
    }
    for symbol in &model.declarations {
        if let Symbol::Enum(enumeration) = symbol {
            let name = type_name(&enumeration.name);
            header.push_str(&format!("typedef int32_t {name}_t;\n"));
            for value in &enumeration.values {
                header.push_str(&format!(
                    "#define {} INT32_C({})\n",
                    upper_snake(&value.name),
                    value.number
                ));
            }
            header.push('\n');
        }
    }
    for message in &messages {
        emit_message_definition(&mut header, message);
        header.push('\n');
    }
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!(
            "#define {}_MESSAGE_ID {}U\n",
            upper_snake(&message.name),
            message.id
        ));
        header.push_str(&format!("void {name}_clear({name}_t *value);\n"));
        header.push_str(&format!(
            "size_t {name}_encoded_size(const {name}_t *value);\n"
        ));
        header.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length);\n"));
        header.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out);\n\n"));
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    let source = emit_source(&module, &messages);
    Ok(GeneratedC { header, source })
}

fn emit_message_definition(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    output.push_str(&format!("struct {name} {{\n"));
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let ty = c_type(&field.ty);
        match field.cardinality {
            crate::ast::Cardinality::Optional => {
                output.push_str(&format!("  bool has_{field_name};\n  {ty} {field_name};\n"));
            }
            crate::ast::Cardinality::Repeated => {
                output.push_str(&format!("  {ty} *{field_name};\n  size_t {field_name}_count;\n  size_t {field_name}_capacity;\n"));
            }
        }
    }
    output.push_str("};\n");
}

fn emit_source(module: &str, messages: &[&MessageSymbol]) -> String {
    let mut source = format!("#include \"{module}.h\"\n\n#include <stdint.h>\n\n");
    source.push_str("/* The body codec is emitted in the next generator milestone. */\n");
    for message in messages {
        let name = type_name(&message.name);
        source.push_str(&format!(
            "void {name}_clear({name}_t *value) {{\n  if (value == NULL) return;\n"
        ));
        for field in &message.fields {
            let field_name = c_identifier(&field.name);
            match field.cardinality {
                crate::ast::Cardinality::Optional => {
                    source.push_str(&format!("  value->has_{field_name} = false;\n"));
                    source.push_str(&format!(
                        "  value->{field_name} = {};\n",
                        default_c_value(field)
                    ));
                }
                crate::ast::Cardinality::Repeated => {
                    source.push_str(&format!("  value->{field_name}_count = 0;\n"))
                }
            }
        }
        source.push_str("}\n\n");
        source.push_str(&format!("size_t {name}_encoded_size(const {name}_t *value) {{ (void)value; return SIZE_MAX; }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length) {{ (void)value; (void)out; (void)out_capacity; (void)out_length; return WL_CODEC_ERR_INVALID_VALUE; }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out) {{ (void)input; (void)input_length; (void)out; return WL_CODEC_ERR_INVALID_VALUE; }}\n\n"));
    }
    source
}

fn default_c_value(field: &FieldSymbol) -> String {
    match (&field.ty, &field.default) {
        (_, Some(FieldDefault::Bool(value))) => value.to_string(),
        (_, Some(FieldDefault::String(value))) => format!(
            "(wl_codec_string_t){{ {}, {}U }}",
            c_string(value),
            value.len()
        ),
        (_, Some(FieldDefault::Int32(value)) | Some(FieldDefault::Enum(value))) => {
            format!("INT32_C({value})")
        }
        (_, Some(FieldDefault::Uint32(value)) | Some(FieldDefault::Fixed32(value))) => {
            format!("UINT32_C({value})")
        }
        (_, Some(FieldDefault::Int64(value))) => format!("INT64_C({value})"),
        (_, Some(FieldDefault::Uint64(value)) | Some(FieldDefault::Fixed64(value))) => {
            format!("UINT64_C({value})")
        }
        (ResolvedType::Bytes, _) => "(wl_codec_bytes_t){ NULL, 0U }".to_owned(),
        (ResolvedType::String, _) => "(wl_codec_string_t){ NULL, 0U }".to_owned(),
        (ResolvedType::Message { name, .. }, _) => format!("({}_t){{0}}", type_name(name)),
        _ => "0".to_owned(),
    }
}

fn c_string(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\"\\x{byte:02X}\""))
        .collect()
}

fn c_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Bool => "bool".to_owned(),
        ResolvedType::Bytes => "wl_codec_bytes_t".to_owned(),
        ResolvedType::String => "wl_codec_string_t".to_owned(),
        ResolvedType::Int32 => "int32_t".to_owned(),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => "uint32_t".to_owned(),
        ResolvedType::Int64 => "int64_t".to_owned(),
        ResolvedType::Uint64 | ResolvedType::Fixed64 => "uint64_t".to_owned(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => {
            format!("{}_t", type_name(name))
        }
    }
}

fn ordered_messages(model: &SemanticModel) -> Result<Vec<&MessageSymbol>, CodegenError> {
    let messages: HashMap<&str, &MessageSymbol> = model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.name.as_str(), message)),
            _ => None,
        })
        .collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    fn visit<'a>(
        message: &'a MessageSymbol,
        messages: &HashMap<&'a str, &'a MessageSymbol>,
        emitted: &mut BTreeSet<&'a str>,
        ordered: &mut Vec<&'a MessageSymbol>,
    ) {
        if !emitted.insert(message.name.as_str()) {
            return;
        }
        for field in &message.fields {
            if let ResolvedType::Message { name, .. } = &field.ty {
                if let Some(child) = messages.get(name.as_str()) {
                    visit(child, messages, emitted, ordered);
                }
            }
        }
        ordered.push(message);
    }
    for symbol in &model.declarations {
        if let Symbol::Message(message) = symbol {
            visit(message, &messages, &mut emitted, &mut ordered);
        }
    }
    Ok(ordered)
}

fn validate_names(model: &SemanticModel) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for symbol in &model.declarations {
        let name = type_name(symbol.name());
        if !names.insert(name.clone()) {
            return Err(CodegenError(format!(
                "declarations collide as C identifier `{name}`"
            )));
        }
    }
    Ok(())
}

fn type_name(name: &str) -> String {
    c_identifier(name)
}
fn c_identifier(name: &str) -> String {
    let mut output: String = name
        .to_snake_case()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    if output.is_empty() {
        return output;
    }
    if output.as_bytes()[0].is_ascii_digit() {
        output.insert(0, '_');
    }
    if is_c_keyword(&output) {
        output.push('_');
    }
    output
}
fn upper_snake(name: &str) -> String {
    c_identifier(name).to_shouty_snake_case()
}

fn is_c_keyword(name: &str) -> bool {
    matches!(
        name,
        "auto"
            | "break"
            | "case"
            | "char"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extern"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "register"
            | "restrict"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "typedef"
            | "union"
            | "unsigned"
            | "void"
            | "volatile"
            | "while"
            | "_bool"
            | "_complex"
            | "_imaginary"
    )
}
