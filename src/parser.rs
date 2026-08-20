use std::collections::HashSet;

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::{
    ast::{
        Cardinality, Declaration, Enum, EnumValue, Field, IntegerLiteral, Literal, Message, Schema,
        Span, Spanned,
    },
    lexer::{LexError, Token, TokenKind, tokenize},
};

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("{message}")]
#[diagnostic(code(wlc::schema))]
pub struct ParseError {
    #[label("{message}")]
    source_span: SourceSpan,
    pub span: Span,
    pub message: String,
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self::new(error.span, error.message)
    }
}

impl ParseError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            source_span: (span.offset, span.length).into(),
            span,
            message: message.into(),
        }
    }
}

/// Parses and validates a Wirelink schema source file.
pub fn parse_schema(source: &str) -> Result<Schema, ParseError> {
    Parser::new(tokenize(source)?).parse_schema()
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn parse_schema(&mut self) -> Result<Schema, ParseError> {
        let version_span = self.current().span;
        self.expect_keyword(TokenKind::Version, "`version`")?;
        let version = self.positive_u32("schema version")?;
        self.expect_symbol(TokenKind::Semicolon, "`;` after version")?;

        let mut declarations = Vec::new();
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        let mut reserved_ids = Vec::new();
        let mut reserved_id_values = HashSet::new();
        while !matches!(self.current().kind, TokenKind::End) {
            if matches!(self.current().kind, TokenKind::Reserved) {
                let reserved = self.parse_reservation("declaration ID")?;
                if !reserved_id_values.insert(reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!("duplicate reserved declaration ID {}", reserved.value),
                    ));
                }
                if ids.contains(&reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!(
                            "declaration ID {} is both active and reserved",
                            reserved.value
                        ),
                    ));
                }
                reserved_ids.push(reserved);
                continue;
            }
            let declaration = match self.current().kind {
                TokenKind::Message => Declaration::Message(self.parse_message()?),
                TokenKind::Enum => Declaration::Enum(self.parse_enum()?),
                _ => {
                    return Err(
                        self.error_current("expected `message`, `enum`, or `reserved` declaration")
                    );
                }
            };
            if !names.insert(declaration.name().value.clone()) {
                return Err(self.error(
                    declaration.name().span,
                    format!("duplicate declaration `{}`", declaration.name().value),
                ));
            }
            if !ids.insert(declaration.id().value) {
                return Err(self.error(
                    declaration.id().span,
                    format!("duplicate declaration id {}", declaration.id().value),
                ));
            }
            if reserved_id_values.contains(&declaration.id().value) {
                return Err(self.error(
                    declaration.id().span,
                    format!(
                        "declaration ID {} is both active and reserved",
                        declaration.id().value
                    ),
                ));
            }
            declarations.push(declaration);
        }

        if declarations.is_empty() {
            return Err(self.error(
                version_span,
                "a schema must declare at least one message or enum",
            ));
        }
        Ok(Schema {
            version,
            reserved_ids,
            declarations,
        })
    }

    fn parse_message(&mut self) -> Result<Message, ParseError> {
        self.expect_keyword(TokenKind::Message, "`message`")?;
        let name = self.identifier("message name")?;
        self.expect_symbol(TokenKind::Equal, "`=` after message name")?;
        let id = self.positive_u16("message id")?;
        self.expect_symbol(TokenKind::LeftBrace, "`{` before message fields")?;

        let mut fields = Vec::new();
        let mut names = HashSet::new();
        let mut numbers = HashSet::new();
        let mut reserved_numbers = Vec::new();
        let mut reserved_number_values = HashSet::new();
        while !matches!(self.current().kind, TokenKind::RightBrace) {
            if matches!(self.current().kind, TokenKind::End) {
                return Err(self.error_current("expected `}` to close message"));
            }
            if matches!(self.current().kind, TokenKind::Reserved) {
                let reserved = self.parse_reservation("field number")?;
                if !reserved_number_values.insert(reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!("duplicate reserved field number {}", reserved.value),
                    ));
                }
                if numbers.contains(&reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!(
                            "field number {} is both active and reserved",
                            reserved.value
                        ),
                    ));
                }
                reserved_numbers.push(reserved);
                continue;
            }
            let field = self.parse_field()?;
            if !names.insert(field.name.value.clone()) {
                return Err(self.error(
                    field.name.span,
                    format!("duplicate field `{}`", field.name.value),
                ));
            }
            if !numbers.insert(field.number.value) {
                return Err(self.error(
                    field.number.span,
                    format!("duplicate field number {}", field.number.value),
                ));
            }
            if reserved_number_values.contains(&field.number.value) {
                return Err(self.error(
                    field.number.span,
                    format!(
                        "field number {} is both active and reserved",
                        field.number.value
                    ),
                ));
            }
            fields.push(field);
        }
        self.advance();
        Ok(Message {
            name,
            id,
            reserved_numbers,
            fields,
        })
    }

    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let cardinality = match self.current().kind {
            TokenKind::Optional => {
                self.advance();
                Cardinality::Optional
            }
            TokenKind::Repeated => {
                self.advance();
                Cardinality::Repeated
            }
            _ => return Err(self.error_current("field must start with `optional` or `repeated`")),
        };
        let ty = self.identifier("field type")?;
        let name = self.identifier("field name")?;
        self.expect_symbol(TokenKind::Equal, "`=` after field name")?;
        let number = self.positive_u16("field number")?;
        let default = if matches!(self.current().kind, TokenKind::LeftBracket) {
            self.advance();
            if cardinality == Cardinality::Repeated {
                return Err(self.error_current("repeated fields cannot declare a default value"));
            }
            self.expect_keyword(TokenKind::Default, "`default` option")?;
            self.expect_symbol(TokenKind::Equal, "`=` after `default`")?;
            let default = self.literal()?;
            self.expect_symbol(TokenKind::RightBracket, "`]` after field option")?;
            Some(default)
        } else {
            None
        };
        self.expect_symbol(TokenKind::Semicolon, "`;` after field")?;
        Ok(Field {
            cardinality,
            ty,
            name,
            number,
            default,
        })
    }

    fn parse_reservation(&mut self, description: &str) -> Result<Spanned<u16>, ParseError> {
        self.expect_keyword(TokenKind::Reserved, "`reserved`")?;
        let number = self.positive_u16(description)?;
        self.expect_symbol(TokenKind::Semicolon, "`;` after reserved ID")?;
        Ok(number)
    }

    fn parse_i32_reservation(&mut self, description: &str) -> Result<Spanned<i32>, ParseError> {
        self.expect_keyword(TokenKind::Reserved, "`reserved`")?;
        let number = self.i32(description)?;
        self.expect_symbol(TokenKind::Semicolon, "`;` after reserved ID")?;
        Ok(number)
    }

    fn parse_enum(&mut self) -> Result<Enum, ParseError> {
        self.expect_keyword(TokenKind::Enum, "`enum`")?;
        let name = self.identifier("enum name")?;
        self.expect_symbol(TokenKind::Equal, "`=` after enum name")?;
        let id = self.positive_u16("enum id")?;
        self.expect_symbol(TokenKind::LeftBrace, "`{` before enum values")?;

        let mut values = Vec::new();
        let mut names = HashSet::new();
        let mut numbers = HashSet::new();
        let mut reserved_numbers = Vec::new();
        let mut reserved_number_values = HashSet::new();
        while !matches!(self.current().kind, TokenKind::RightBrace) {
            if matches!(self.current().kind, TokenKind::End) {
                return Err(self.error_current("expected `}` to close enum"));
            }
            if matches!(self.current().kind, TokenKind::Reserved) {
                let reserved = self.parse_i32_reservation("enum value")?;
                if !reserved_number_values.insert(reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!("duplicate reserved enum value {}", reserved.value),
                    ));
                }
                if numbers.contains(&reserved.value) {
                    return Err(self.error(
                        reserved.span,
                        format!("enum value {} is both active and reserved", reserved.value),
                    ));
                }
                reserved_numbers.push(reserved);
                continue;
            }
            let value_name = self.identifier("enum value name")?;
            self.expect_symbol(TokenKind::Equal, "`=` after enum value name")?;
            let number = self.i32("enum value")?;
            self.expect_symbol(TokenKind::Semicolon, "`;` after enum value")?;
            if !names.insert(value_name.value.clone()) {
                return Err(self.error(
                    value_name.span,
                    format!("duplicate enum value `{}`", value_name.value),
                ));
            }
            if !numbers.insert(number.value) {
                return Err(self.error(
                    number.span,
                    format!("duplicate enum value {}", number.value),
                ));
            }
            if reserved_number_values.contains(&number.value) {
                return Err(self.error(
                    number.span,
                    format!("enum value {} is both active and reserved", number.value),
                ));
            }
            values.push(EnumValue {
                name: value_name,
                number,
            });
        }
        self.advance();
        if values.is_empty() {
            return Err(self.error(name.span, "an enum must declare at least one value"));
        }
        Ok(Enum {
            name,
            id,
            reserved_numbers,
            values,
        })
    }

    fn identifier(&mut self, expected: &str) -> Result<Spanned<String>, ParseError> {
        let token = self.current().clone();
        if let TokenKind::Identifier(value) = token.kind {
            self.advance();
            Ok(Spanned {
                value,
                span: token.span,
            })
        } else {
            Err(self.error(token.span, format!("expected {expected}")))
        }
    }

    fn literal(&mut self) -> Result<Spanned<Literal>, ParseError> {
        let token = self.current().clone();
        let value = match token.kind {
            TokenKind::Number(value) => Literal::Integer(value),
            TokenKind::String(value) => Literal::String(value),
            TokenKind::True => Literal::Boolean(true),
            TokenKind::False => Literal::Boolean(false),
            _ => {
                return Err(self.error(
                    token.span,
                    "expected integer, string, or boolean default value",
                ));
            }
        };
        self.advance();
        Ok(Spanned {
            value,
            span: token.span,
        })
    }

    fn positive_u16(&mut self, description: &str) -> Result<Spanned<u16>, ParseError> {
        let value = self.integer(description)?;
        let span = value.span;
        match value
            .value
            .as_u32()
            .and_then(|value| u16::try_from(value).ok())
        {
            Some(value) if value > 0 => Ok(Spanned { value, span }),
            _ => Err(self.error(value.span, format!("{description} must be in 1..=65535"))),
        }
    }

    fn positive_u32(&mut self, description: &str) -> Result<Spanned<u32>, ParseError> {
        let value = self.integer(description)?;
        let span = value.span;
        match value.value.as_u32() {
            Some(value) if value > 0 => Ok(Spanned { value, span }),
            _ => Err(self.error(
                value.span,
                format!("{description} must be in 1..=4294967295"),
            )),
        }
    }

    fn i32(&mut self, description: &str) -> Result<Spanned<i32>, ParseError> {
        let value = self.integer(description)?;
        let span = value.span;
        value
            .value
            .as_i32()
            .map(|value| Spanned { value, span })
            .ok_or_else(|| self.error(span, format!("{description} is outside the i32 range")))
    }

    fn integer(&mut self, description: &str) -> Result<Spanned<IntegerLiteral>, ParseError> {
        let token = self.current().clone();
        if let TokenKind::Number(value) = token.kind {
            self.advance();
            Ok(Spanned {
                value,
                span: token.span,
            })
        } else {
            Err(self.error(token.span, format!("expected {description}")))
        }
    }

    fn expect_keyword(&mut self, expected: TokenKind, description: &str) -> Result<(), ParseError> {
        self.expect_symbol(expected, description)
    }
    fn expect_symbol(&mut self, expected: TokenKind, description: &str) -> Result<(), ParseError> {
        if std::mem::discriminant(&self.current().kind) == std::mem::discriminant(&expected) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_current(format!("expected {description}")))
        }
    }
    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }
    fn advance(&mut self) {
        self.position += 1;
    }
    fn error_current(&self, message: impl Into<String>) -> ParseError {
        self.error(self.current().span, message)
    }
    fn error(&self, span: Span, message: impl Into<String>) -> ParseError {
        ParseError::new(span, message)
    }
}
