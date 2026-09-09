// SPDX-License-Identifier: Apache-2.0
use super::CodegenError;
use crate::semantic::{MessageSymbol, ResolvedType, SemanticModel, Symbol};
use heck::{ToShoutySnakeCase, ToSnakeCase};
use std::collections::{BTreeSet, HashMap};

pub(super) fn c_string(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\"\\x{byte:02X}\""))
        .collect()
}

pub(crate) fn c_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Bool => "bool".to_owned(),
        ResolvedType::Bytes => "wl_codec_bytes_t".to_owned(),
        ResolvedType::String => "wl_codec_string_t".to_owned(),
        ResolvedType::Int8 => "int8_t".to_owned(),
        ResolvedType::Uint8 => "uint8_t".to_owned(),
        ResolvedType::Int16 => "int16_t".to_owned(),
        ResolvedType::Uint16 => "uint16_t".to_owned(),
        ResolvedType::Int32 => "int32_t".to_owned(),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => "uint32_t".to_owned(),
        ResolvedType::Float32 => "float".to_owned(),
        ResolvedType::Int64 => "int64_t".to_owned(),
        ResolvedType::Uint64 | ResolvedType::Fixed64 => "uint64_t".to_owned(),
        ResolvedType::Float64 => "double".to_owned(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => {
            format!("{}_t", type_name(name))
        }
    }
}

pub(super) fn ieee_float_usage(model: &SemanticModel) -> (bool, bool) {
    let mut float32 = false;
    let mut float64 = false;
    for field in model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some(message.fields.as_slice()),
            Symbol::Enum(_) => None,
        })
        .flatten()
    {
        float32 |= matches!(field.ty, ResolvedType::Float32);
        float64 |= matches!(field.ty, ResolvedType::Float64);
    }
    (float32, float64)
}

pub(super) fn ordered_messages(model: &SemanticModel) -> Result<Vec<&MessageSymbol>, CodegenError> {
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
            if let ResolvedType::Message { name, .. } = &field.ty
                && let Some(child) = messages.get(name.as_str())
            {
                visit(child, messages, emitted, ordered);
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

pub(super) fn validate_names(
    model: &SemanticModel,
    maxima: &HashMap<u16, Option<u64>>,
) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for symbol in &model.declarations {
        let name = type_name(symbol.name());
        if !names.insert(name.clone()) {
            return Err(CodegenError(format!(
                "declarations collide as C identifier `{name}`"
            )));
        }
    }
    let mut macros = BTreeSet::new();
    for symbol in &model.declarations {
        match symbol {
            Symbol::Message(message) => {
                let prefix = upper_snake(&message.name);
                let mut generated = vec![
                    format!("{prefix}_MESSAGE_ID"),
                    format!("{prefix}_HAS_MAX_ENCODED_SIZE"),
                    format!("{prefix}_HAS_VALUE"),
                ];
                if maxima.get(&message.id).copied().flatten().is_some() {
                    generated.push(format!("{prefix}_MAX_ENCODED_SIZE"));
                    generated.push(format!("{prefix}_VALUE_SIZE"));
                    let name = format!("{}_value", type_name(&message.name));
                    if names.contains(&name) {
                        return Err(CodegenError(format!(
                            "declaration collides with generated value `{name}_t`"
                        )));
                    }
                }
                for name in generated {
                    if !macros.insert(name.clone()) {
                        return Err(CodegenError(format!(
                            "generated macros collide as C identifier `{name}`"
                        )));
                    }
                }
            }
            Symbol::Enum(enumeration) => {
                for value in &enumeration.values {
                    let name = upper_snake(&value.name);
                    if !macros.insert(name.clone()) {
                        return Err(CodegenError(format!(
                            "generated macros collide as C identifier `{name}`"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn type_name(name: &str) -> String {
    c_identifier(name)
}
pub(crate) fn c_identifier(name: &str) -> String {
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
pub(crate) fn upper_snake(name: &str) -> String {
    c_identifier(name).to_shouty_snake_case()
}

pub(super) fn is_c_keyword(name: &str) -> bool {
    matches!(
        name,
        /* C11 keywords, stdbool macros, and C++20 keywords. Generated public
         * headers are required to compile as both strict C11 and C++20. */
        "alignas"
            | "alignof"
            | "and"
            | "and_eq"
            | "asm"
            | "atomic_cancel"
            | "atomic_commit"
            | "atomic_noexcept"
            | "auto"
            | "bitand"
            | "bitor"
            | "bool"
            | "break"
            | "case"
            | "catch"
            | "char"
            | "char8_t"
            | "char16_t"
            | "char32_t"
            | "class"
            | "compl"
            | "concept"
            | "const"
            | "consteval"
            | "constexpr"
            | "constinit"
            | "const_cast"
            | "continue"
            | "co_await"
            | "co_return"
            | "co_yield"
            | "decltype"
            | "default"
            | "delete"
            | "do"
            | "double"
            | "dynamic_cast"
            | "else"
            | "enum"
            | "explicit"
            | "extern"
            | "export"
            | "false"
            | "float"
            | "for"
            | "friend"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "mutable"
            | "namespace"
            | "new"
            | "noexcept"
            | "not"
            | "not_eq"
            | "nullptr"
            | "operator"
            | "or"
            | "or_eq"
            | "private"
            | "protected"
            | "public"
            | "register"
            | "reinterpret_cast"
            | "requires"
            | "restrict"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "static_assert"
            | "static_cast"
            | "struct"
            | "switch"
            | "template"
            | "this"
            | "thread_local"
            | "throw"
            | "true"
            | "try"
            | "typeid"
            | "typename"
            | "typedef"
            | "union"
            | "unsigned"
            | "using"
            | "virtual"
            | "void"
            | "volatile"
            | "wchar_t"
            | "while"
            | "xor"
            | "xor_eq"
            | "_alignas"
            | "_alignof"
            | "_atomic"
            | "_bool"
            | "_complex"
            | "_generic"
            | "_imaginary"
            | "_noreturn"
            | "_static_assert"
            | "_thread_local"
    )
}
