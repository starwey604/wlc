use std::collections::{BTreeSet, HashMap};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast::{Cardinality, Declaration, Literal, Schema, Span, Spanned};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticModel {
    pub version: u32,
    pub reserved_ids: BTreeSet<u16>,
    pub declarations: Vec<Symbol>,
    version_span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Symbol {
    Message(MessageSymbol),
    Enum(EnumSymbol),
}

impl Symbol {
    pub fn id(&self) -> u16 {
        match self {
            Self::Message(message) => message.id,
            Self::Enum(enumeration) => enumeration.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Message(message) => &message.name,
            Self::Enum(enumeration) => &enumeration.name,
        }
    }

    fn span(&self) -> Span {
        match self {
            Self::Message(message) => message.span,
            Self::Enum(enumeration) => enumeration.span,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageSymbol {
    pub name: String,
    pub id: u16,
    pub reserved_numbers: BTreeSet<u16>,
    pub fields: Vec<FieldSymbol>,
    span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldSymbol {
    pub name: String,
    pub number: u16,
    pub cardinality: Cardinality,
    pub ty: ResolvedType,
    pub default: Option<FieldDefault>,
    span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumSymbol {
    pub name: String,
    pub id: u16,
    pub reserved_numbers: BTreeSet<i32>,
    pub values: Vec<EnumValueSymbol>,
    span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumValueSymbol {
    pub name: String,
    pub number: i32,
    span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedType {
    Bool,
    Bytes,
    String,
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Int64,
    Uint64,
    Fixed32,
    Fixed64,
    Float32,
    Float64,
    Message { id: u16, name: String },
    Enum { id: u16, name: String },
}

/// A type-checked optional-field default suitable for code generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldDefault {
    Bool(bool),
    String(String),
    Int8(i8),
    Uint8(u8),
    Int16(i16),
    Uint16(u16),
    Int32(i32),
    Uint32(u32),
    Int64(i64),
    Uint64(u64),
    Fixed32(u32),
    Fixed64(u64),
    Enum(i32),
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("{message}")]
#[diagnostic(code(wlc::semantic))]
pub struct SemanticError {
    #[label("{message}")]
    source_span: SourceSpan,
    pub span: Span,
    pub message: String,
}

impl SemanticError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            source_span: (span.offset, span.length).into(),
            span,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("schema semantic validation failed")]
pub struct SemanticErrors {
    #[related]
    errors: Vec<SemanticError>,
}

impl SemanticErrors {
    pub fn errors(&self) -> &[SemanticError] {
        &self.errors
    }
}

/// Resolves all field types and returns a declaration-order-independent model.
pub fn analyze_schema(schema: &Schema) -> Result<SemanticModel, SemanticErrors> {
    let declarations_by_name: HashMap<&str, &Declaration> = schema
        .declarations
        .iter()
        .map(|declaration| (declaration.name().value.as_str(), declaration))
        .collect();
    let mut errors = Vec::new();
    let mut declarations = Vec::new();

    for declaration in &schema.declarations {
        if is_builtin(declaration.name().value.as_str()) {
            errors.push(SemanticError::new(
                declaration.name().span,
                format!(
                    "declaration name `{}` is a built-in type",
                    declaration.name().value
                ),
            ));
        }
    }

    for declaration in &schema.declarations {
        match declaration {
            Declaration::Message(message) => {
                let mut fields = Vec::new();
                for field in &message.fields {
                    let Some(ty) = resolve_type(&field.ty.value, &declarations_by_name) else {
                        errors.push(SemanticError::new(
                            field.ty.span,
                            format!("unknown field type `{}`", field.ty.value),
                        ));
                        continue;
                    };
                    if matches!(
                        field.cardinality,
                        Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
                    ) && !matches!(
                        &ty,
                        ResolvedType::Float32
                            | ResolvedType::Float64
                            | ResolvedType::Fixed32
                            | ResolvedType::Fixed64
                    ) {
                        errors.push(SemanticError::new(
                            field.ty.span,
                            format!(
                                "packed field `{}` must use float32, float64, fixed32, or fixed64",
                                field.name.value
                            ),
                        ));
                    }
                    let default = match field.default.as_ref() {
                        Some(default) if cardinality_is_required(field.cardinality) => {
                            errors.push(SemanticError::new(
                                default.span,
                                "required fields cannot declare a default value",
                            ));
                            None
                        }
                        _ => lower_default(
                            field.default.as_ref(),
                            &ty,
                            &declarations_by_name,
                            &mut errors,
                        ),
                    };
                    fields.push(FieldSymbol {
                        name: field.name.value.clone(),
                        number: field.number.value,
                        cardinality: field.cardinality,
                        ty,
                        default,
                        span: field.number.span,
                    });
                }
                fields.sort_by_key(|field| field.number);
                declarations.push(Symbol::Message(MessageSymbol {
                    name: message.name.value.clone(),
                    id: message.id.value,
                    reserved_numbers: message
                        .reserved_numbers
                        .iter()
                        .map(|number| number.value)
                        .collect(),
                    fields,
                    span: message.name.span,
                }));
            }
            Declaration::Enum(enumeration) => {
                let mut values: Vec<_> = enumeration
                    .values
                    .iter()
                    .map(|value| EnumValueSymbol {
                        name: value.name.value.clone(),
                        number: value.number.value,
                        span: value.number.span,
                    })
                    .collect();
                values.sort_by_key(|value| value.number);
                declarations.push(Symbol::Enum(EnumSymbol {
                    name: enumeration.name.value.clone(),
                    id: enumeration.id.value,
                    reserved_numbers: enumeration
                        .reserved_numbers
                        .iter()
                        .map(|number| number.value)
                        .collect(),
                    values,
                    span: enumeration.name.span,
                }));
            }
        }
    }

    validate_message_nesting(&declarations, &mut errors);
    if !errors.is_empty() {
        return Err(SemanticErrors { errors });
    }
    declarations.sort_by_key(Symbol::id);
    Ok(SemanticModel {
        version: schema.version.value,
        reserved_ids: schema.reserved_ids.iter().map(|id| id.value).collect(),
        declarations,
        version_span: schema.version.span,
    })
}

/// Validates wire compatibility from `previous` to `current`.
pub fn check_compatibility(
    previous: &SemanticModel,
    current: &SemanticModel,
) -> Result<(), SemanticErrors> {
    let mut errors = Vec::new();
    if current.version <= previous.version {
        errors.push(SemanticError::new(
            current.version_span,
            format!(
                "schema revision must increase from {} to a value greater than it",
                previous.version
            ),
        ));
    }
    for previous_symbol in &previous.declarations {
        match current.symbol_by_id(previous_symbol.id()) {
            None if !current.reserved_ids.contains(&previous_symbol.id()) => {
                errors.push(SemanticError::new(
                    current.version_span,
                    format!(
                        "removed declaration `{}` (ID {}) must be retained as `reserved {};`",
                        previous_symbol.name(),
                        previous_symbol.id(),
                        previous_symbol.id()
                    ),
                ))
            }
            None => {}
            Some(current_symbol) => {
                if std::mem::discriminant(previous_symbol) != std::mem::discriminant(current_symbol)
                {
                    errors.push(SemanticError::new(
                        current_symbol.span(),
                        format!("declaration ID {} changed kind", previous_symbol.id()),
                    ));
                }
                if previous_symbol.name() != current_symbol.name() {
                    errors.push(SemanticError::new(
                        current_symbol.span(),
                        format!(
                            "declaration ID {} changed name from `{}` to `{}`",
                            previous_symbol.id(),
                            previous_symbol.name(),
                            current_symbol.name()
                        ),
                    ));
                }
                match (previous_symbol, current_symbol) {
                    (Symbol::Message(previous_message), Symbol::Message(current_message)) => {
                        check_message_compatibility(previous_message, current_message, &mut errors)
                    }
                    (Symbol::Enum(previous_enum), Symbol::Enum(current_enum)) => {
                        check_enum_compatibility(previous_enum, current_enum, &mut errors)
                    }
                    _ => {}
                }
            }
        }
        if let Some(current_symbol) = current.symbol_by_name(previous_symbol.name())
            && current_symbol.id() != previous_symbol.id()
        {
            errors.push(SemanticError::new(
                current_symbol.span(),
                format!(
                    "declaration `{}` changed ID from {} to {}",
                    previous_symbol.name(),
                    previous_symbol.id(),
                    current_symbol.id()
                ),
            ));
        }
    }
    for reserved_id in &previous.reserved_ids {
        if !current.reserved_ids.contains(reserved_id) {
            errors.push(SemanticError::new(
                current.version_span,
                format!("reserved declaration ID {reserved_id} must remain reserved"),
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SemanticErrors { errors })
    }
}

impl SemanticModel {
    fn symbol_by_id(&self, id: u16) -> Option<&Symbol> {
        self.declarations.iter().find(|symbol| symbol.id() == id)
    }

    fn symbol_by_name(&self, name: &str) -> Option<&Symbol> {
        self.declarations
            .iter()
            .find(|symbol| symbol.name() == name)
    }
}

fn resolve_type(name: &str, declarations: &HashMap<&str, &Declaration>) -> Option<ResolvedType> {
    let builtin = match name {
        "bool" => Some(ResolvedType::Bool),
        "bytes" => Some(ResolvedType::Bytes),
        "string" => Some(ResolvedType::String),
        "int8" => Some(ResolvedType::Int8),
        "uint8" => Some(ResolvedType::Uint8),
        "int16" => Some(ResolvedType::Int16),
        "uint16" => Some(ResolvedType::Uint16),
        "int32" => Some(ResolvedType::Int32),
        "uint32" => Some(ResolvedType::Uint32),
        "int64" => Some(ResolvedType::Int64),
        "uint64" => Some(ResolvedType::Uint64),
        "fixed32" => Some(ResolvedType::Fixed32),
        "fixed64" => Some(ResolvedType::Fixed64),
        "float32" => Some(ResolvedType::Float32),
        "float64" => Some(ResolvedType::Float64),
        _ => None,
    };
    builtin.or_else(|| match declarations.get(name) {
        Some(Declaration::Message(message)) => Some(ResolvedType::Message {
            id: message.id.value,
            name: message.name.value.clone(),
        }),
        Some(Declaration::Enum(enumeration)) => Some(ResolvedType::Enum {
            id: enumeration.id.value,
            name: enumeration.name.value.clone(),
        }),
        None => None,
    })
}

fn lower_default(
    default: Option<&Spanned<Literal>>,
    ty: &ResolvedType,
    declarations: &HashMap<&str, &Declaration>,
    errors: &mut Vec<SemanticError>,
) -> Option<FieldDefault> {
    let default = default?;
    let invalid = |errors: &mut Vec<SemanticError>, message: String| {
        errors.push(SemanticError::new(default.span, message));
        None
    };
    match (ty, &default.value) {
        (ResolvedType::Bool, Literal::Boolean(value)) => Some(FieldDefault::Bool(*value)),
        (ResolvedType::String, Literal::String(value)) => Some(FieldDefault::String(value.clone())),
        (ResolvedType::Bytes, _) => {
            invalid(errors, "bytes fields cannot declare defaults".to_owned())
        }
        (ResolvedType::Int8, Literal::Integer(value)) => value
            .as_i8()
            .map(FieldDefault::Int8)
            .or_else(|| invalid(errors, format!("default value {value} does not fit int8"))),
        (ResolvedType::Uint8, Literal::Integer(value)) => value
            .as_u8()
            .map(FieldDefault::Uint8)
            .or_else(|| invalid(errors, format!("default value {value} does not fit uint8"))),
        (ResolvedType::Int16, Literal::Integer(value)) => value
            .as_i16()
            .map(FieldDefault::Int16)
            .or_else(|| invalid(errors, format!("default value {value} does not fit int16"))),
        (ResolvedType::Uint16, Literal::Integer(value)) => value
            .as_u16()
            .map(FieldDefault::Uint16)
            .or_else(|| invalid(errors, format!("default value {value} does not fit uint16"))),
        (ResolvedType::Int32, Literal::Integer(value)) => value
            .as_i32()
            .map(FieldDefault::Int32)
            .or_else(|| invalid(errors, format!("default value {value} does not fit int32"))),
        (ResolvedType::Uint32, Literal::Integer(value)) => value
            .as_u32()
            .map(FieldDefault::Uint32)
            .or_else(|| invalid(errors, format!("default value {value} does not fit uint32"))),
        (ResolvedType::Int64, Literal::Integer(value)) => value
            .as_i64()
            .map(FieldDefault::Int64)
            .or_else(|| invalid(errors, format!("default value {value} does not fit int64"))),
        (ResolvedType::Uint64, Literal::Integer(value)) => value
            .as_u64()
            .map(FieldDefault::Uint64)
            .or_else(|| invalid(errors, format!("default value {value} does not fit uint64"))),
        (ResolvedType::Fixed32, Literal::Integer(value)) => {
            value.as_u32().map(FieldDefault::Fixed32).or_else(|| {
                invalid(
                    errors,
                    format!("default value {value} does not fit fixed32"),
                )
            })
        }
        (ResolvedType::Fixed64, Literal::Integer(value)) => {
            value.as_u64().map(FieldDefault::Fixed64).or_else(|| {
                invalid(
                    errors,
                    format!("default value {value} does not fit fixed64"),
                )
            })
        }
        (ResolvedType::Float32, _) => invalid(
            errors,
            "float32 fields cannot declare defaults; use explicit presence and a runtime value"
                .to_owned(),
        ),
        (ResolvedType::Float64, _) => invalid(
            errors,
            "float64 fields cannot declare defaults; use explicit presence and a runtime value"
                .to_owned(),
        ),
        (ResolvedType::Enum { name, .. }, Literal::Integer(value)) => {
            let Some(value) = value.as_i32() else {
                return invalid(
                    errors,
                    format!("default value {value} does not fit enum `{name}`"),
                );
            };
            let declared = matches!(declarations.get(name.as_str()), Some(Declaration::Enum(enumeration)) if enumeration.values.iter().any(|candidate| candidate.number.value == value));
            if declared {
                Some(FieldDefault::Enum(value))
            } else {
                invalid(
                    errors,
                    format!("default value {value} is not declared by enum `{name}`"),
                )
            }
        }
        (ResolvedType::Message { .. }, _) => {
            invalid(errors, "message fields cannot declare defaults".to_owned())
        }
        _ => invalid(
            errors,
            "default value does not match the declared field type".to_owned(),
        ),
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "bytes"
            | "string"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
            | "fixed32"
            | "fixed64"
            | "float32"
            | "float64"
    )
}

fn validate_message_nesting(declarations: &[Symbol], errors: &mut Vec<SemanticError>) {
    let messages: HashMap<u16, &MessageSymbol> = declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.id, message)),
            Symbol::Enum(_) => None,
        })
        .collect();
    for message in messages.values() {
        let mut path = vec![message.id];
        validate_message_children(message, 0, &messages, &mut path, errors);
    }
}

fn validate_message_children(
    message: &MessageSymbol,
    depth: usize,
    messages: &HashMap<u16, &MessageSymbol>,
    path: &mut Vec<u16>,
    errors: &mut Vec<SemanticError>,
) {
    for field in &message.fields {
        let ResolvedType::Message { id, name } = &field.ty else {
            continue;
        };
        if path.contains(id) {
            errors.push(SemanticError::new(
                field.span,
                format!("message nesting is recursive through `{name}`"),
            ));
            continue;
        }
        if depth + 1 > 8 {
            errors.push(SemanticError::new(
                field.span,
                "message nesting depth exceeds the v1 limit of eight",
            ));
            continue;
        }
        if let Some(child) = messages.get(id) {
            path.push(*id);
            validate_message_children(child, depth + 1, messages, path, errors);
            path.pop();
        }
    }
}

fn check_message_compatibility(
    previous: &MessageSymbol,
    current: &MessageSymbol,
    errors: &mut Vec<SemanticError>,
) {
    for reserved in &previous.reserved_numbers {
        if !current.reserved_numbers.contains(reserved) {
            errors.push(SemanticError::new(
                current.span,
                format!(
                    "message `{}` must keep field number {reserved} reserved",
                    current.name
                ),
            ));
        }
    }
    for previous_field in &previous.fields {
        match current.fields.iter().find(|field| field.number == previous_field.number) {
            None if cardinality_is_required(previous_field.cardinality) => errors.push(
                SemanticError::new(
                    current.span,
                    format!(
                        "required field `{}` (number {}) in message `{}` cannot be removed, even if reserved",
                        previous_field.name, previous_field.number, current.name
                    ),
                ),
            ),
            None if !current.reserved_numbers.contains(&previous_field.number) => errors.push(SemanticError::new(
                current.span,
                format!("removed field `{}` (number {}) in message `{}` must be retained as `reserved {};`", previous_field.name, previous_field.number, current.name, previous_field.number),
            )),
            None => {}
            Some(current_field) if current_field.name != previous_field.name || current_field.ty != previous_field.ty || current_field.cardinality != previous_field.cardinality => errors.push(SemanticError::new(
                current_field.span,
                format!("field number {} in message `{}` changed wire identity", previous_field.number, current.name),
            )),
            Some(_) => {}
        }
        if let Some(current_field) = current
            .fields
            .iter()
            .find(|field| field.name == previous_field.name)
            && current_field.number != previous_field.number
        {
            errors.push(SemanticError::new(
                current_field.span,
                format!(
                    "field `{}` in message `{}` changed number from {} to {}",
                    previous_field.name, current.name, previous_field.number, current_field.number
                ),
            ));
        }
    }
    for current_field in &current.fields {
        if cardinality_is_required(current_field.cardinality)
            && !previous
                .fields
                .iter()
                .any(|field| field.number == current_field.number)
        {
            errors.push(SemanticError::new(
                current_field.span,
                format!(
                    "new required field `{}` (number {}) in message `{}` is incompatible",
                    current_field.name, current_field.number, current.name
                ),
            ));
        }
    }
}

fn cardinality_is_required(cardinality: Cardinality) -> bool {
    matches!(
        cardinality,
        Cardinality::Required | Cardinality::RequiredPacked(_)
    )
}

fn check_enum_compatibility(
    previous: &EnumSymbol,
    current: &EnumSymbol,
    errors: &mut Vec<SemanticError>,
) {
    for reserved in &previous.reserved_numbers {
        if !current.reserved_numbers.contains(reserved) {
            errors.push(SemanticError::new(
                current.span,
                format!(
                    "enum `{}` must keep value {reserved} reserved",
                    current.name
                ),
            ));
        }
    }
    for previous_value in &previous.values {
        match current
            .values
            .iter()
            .find(|value| value.number == previous_value.number)
        {
            None if !current.reserved_numbers.contains(&previous_value.number) => {
                errors.push(SemanticError::new(
                    current.span,
                    format!(
                        "removed enum value `{}` ({}) in `{}` must be retained as `reserved {};`",
                        previous_value.name,
                        previous_value.number,
                        current.name,
                        previous_value.number
                    ),
                ))
            }
            None => {}
            Some(current_value) if current_value.name != previous_value.name => {
                errors.push(SemanticError::new(
                    current_value.span,
                    format!(
                        "enum value {} in `{}` changed name",
                        previous_value.number, current.name
                    ),
                ))
            }
            Some(_) => {}
        }
    }
}
