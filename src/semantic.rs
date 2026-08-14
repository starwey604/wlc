use std::collections::{BTreeSet, HashMap};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast::{Cardinality, Declaration, Literal, Schema, Span};

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
    String,
    Int32,
    Uint32,
    Int64,
    Uint64,
    Message { id: u16, name: String },
    Enum { id: u16, name: String },
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
                    validate_default(
                        field.default.as_ref().map(|value| &value.value),
                        &ty,
                        field.ty.span,
                        &declarations_by_name,
                        &mut errors,
                    );
                    fields.push(FieldSymbol {
                        name: field.name.value.clone(),
                        number: field.number.value,
                        cardinality: field.cardinality,
                        ty,
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
        if let Some(current_symbol) = current.symbol_by_name(previous_symbol.name()) {
            if current_symbol.id() != previous_symbol.id() {
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
        "string" => Some(ResolvedType::String),
        "int32" => Some(ResolvedType::Int32),
        "uint32" => Some(ResolvedType::Uint32),
        "int64" => Some(ResolvedType::Int64),
        "uint64" => Some(ResolvedType::Uint64),
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

fn validate_default(
    default: Option<&Literal>,
    ty: &ResolvedType,
    type_span: Span,
    declarations: &HashMap<&str, &Declaration>,
    errors: &mut Vec<SemanticError>,
) {
    match (default, ty) {
        (Some(Literal::Integer(value)), ResolvedType::Enum { name, .. }) => {
            let Some(Declaration::Enum(enumeration)) = declarations.get(name.as_str()) else {
                return;
            };
            if !enumeration
                .values
                .iter()
                .any(|candidate| candidate.number.value as i64 == *value)
            {
                errors.push(SemanticError::new(
                    type_span,
                    format!("default value {value} is not declared by enum `{name}`"),
                ));
            }
        }
        (Some(_), ResolvedType::Message { .. }) => errors.push(SemanticError::new(
            type_span,
            "message fields cannot declare defaults",
        )),
        _ => {}
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
        {
            if current_field.number != previous_field.number {
                errors.push(SemanticError::new(
                    current_field.span,
                    format!(
                        "field `{}` in message `{}` changed number from {} to {}",
                        previous_field.name,
                        current.name,
                        previous_field.number,
                        current_field.number
                    ),
                ));
            }
        }
    }
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
