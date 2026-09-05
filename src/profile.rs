//! Parser and source model for optional application binding profiles.
//!
//! Binding profiles deliberately use a sidecar grammar. They describe local
//! routing and service policy without extending the `.wl` business codec schema.
//! RPC metadata mode also selects a payload wrapper and must match on both peers.

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast::{Span, Spanned};

/// The currently supported binding-profile language revision.
pub const BINDING_PROFILE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingProfile {
    pub version: Spanned<u32>,
    pub bindings: Vec<BindingDeclaration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingDeclaration {
    Latest(RouteBinding),
    Fifo(RouteBinding),
    Rpc(Box<RpcBinding>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteBinding {
    pub message: Spanned<String>,
    pub delivery: Spanned<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcBinding {
    pub name: Spanned<String>,
    pub request: Spanned<String>,
    pub response: Spanned<String>,
    pub request_operation_id: Option<Spanned<String>>,
    pub response_operation_id: Option<Spanned<String>>,
    pub response_status: Option<Spanned<String>>,
    pub request_delivery: Spanned<String>,
    pub response_delivery: Spanned<String>,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("{message}")]
#[diagnostic(code(wlc::profile))]
pub struct ProfileParseError {
    #[label("{message}")]
    source_span: SourceSpan,
    pub span: Span,
    pub message: String,
}

impl ProfileParseError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            source_span: (span.offset, span.length).into(),
            span,
            message: message.into(),
        }
    }
}

/// Parse a versioned binding-profile sidecar.
pub fn parse_binding_profile(source: &str) -> Result<BindingProfile, ProfileParseError> {
    Parser::new(tokenize(source)?).parse_profile()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenKind {
    Identifier(String),
    Number(u32),
    Equal,
    Semicolon,
    LeftBrace,
    RightBrace,
    End,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    span: Span,
}

fn tokenize(source: &str) -> Result<Vec<Token>, ProfileParseError> {
    let mut lexer = Lexer {
        source,
        offset: 0,
        line: 1,
        column: 1,
    };
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let done = token.kind == TokenKind::End;
        tokens.push(token);
        if done {
            return Ok(tokens);
        }
    }
}

struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
}

impl Lexer<'_> {
    fn next_token(&mut self) -> Result<Token, ProfileParseError> {
        self.skip_ignored();
        let span = self.span();
        let Some(character) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::End,
                span,
            });
        };
        let kind = match character {
            '=' => {
                self.advance();
                TokenKind::Equal
            }
            ';' => {
                self.advance();
                TokenKind::Semicolon
            }
            '{' => {
                self.advance();
                TokenKind::LeftBrace
            }
            '}' => {
                self.advance();
                TokenKind::RightBrace
            }
            '0'..='9' => self.read_number(span)?,
            '_' | 'a'..='z' | 'A'..='Z' => self.read_identifier(),
            _ => {
                return Err(ProfileParseError::new(
                    span,
                    format!("unexpected character `{character}`"),
                ));
            }
        };
        Ok(Token {
            kind,
            span: Span {
                length: self.offset - span.offset,
                ..span
            },
        })
    }

    fn skip_ignored(&mut self) {
        loop {
            while matches!(self.peek(), Some(character) if character.is_whitespace()) {
                self.advance();
            }
            if !self.source[self.offset..].starts_with("//") {
                return;
            }
            while !matches!(self.peek(), None | Some('\n')) {
                self.advance();
            }
        }
    }

    fn read_identifier(&mut self) -> TokenKind {
        let start = self.offset;
        self.advance();
        while matches!(self.peek(), Some('_' | 'a'..='z' | 'A'..='Z' | '0'..='9')) {
            self.advance();
        }
        TokenKind::Identifier(self.source[start..self.offset].to_owned())
    }

    fn read_number(&mut self, span: Span) -> Result<TokenKind, ProfileParseError> {
        let start = self.offset;
        while matches!(self.peek(), Some('0'..='9')) {
            self.advance();
        }
        self.source[start..self.offset]
            .parse::<u32>()
            .map(TokenKind::Number)
            .map_err(|_| ProfileParseError::new(span, "integer literal is out of range"))
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn span(&self) -> Span {
        Span {
            offset: self.offset,
            length: 0,
            line: self.line,
            column: self.column,
        }
    }

    fn advance(&mut self) {
        let character = self.peek().expect("advance called at end of input");
        self.offset += character.len_utf8();
        if character == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }
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

    fn parse_profile(&mut self) -> Result<BindingProfile, ProfileParseError> {
        self.expect_word("profile")?;
        self.expect_word("version")?;
        let version = self.positive_u32("profile version")?;
        self.expect_symbol(TokenKind::Semicolon, "`;` after profile version")?;

        let mut bindings = Vec::new();
        while self.current().kind != TokenKind::End {
            let kind = self.word("binding kind")?;
            let binding = match kind.value.as_str() {
                "latest" => BindingDeclaration::Latest(self.parse_route("latest")?),
                "fifo" => BindingDeclaration::Fifo(self.parse_route("fifo")?),
                "rpc" => BindingDeclaration::Rpc(Box::new(self.parse_rpc()?)),
                _ => {
                    return Err(self.error(
                        kind.span,
                        "binding must start with `latest`, `fifo`, or `rpc`",
                    ));
                }
            };
            bindings.push(binding);
        }
        if bindings.is_empty() {
            return Err(self.error(
                version.span,
                "a binding profile must declare at least one binding",
            ));
        }
        Ok(BindingProfile { version, bindings })
    }

    fn parse_route(&mut self, kind: &str) -> Result<RouteBinding, ProfileParseError> {
        let message = self.word(&format!("message name after `{kind}`"))?;
        self.expect_symbol(TokenKind::LeftBrace, "`{` before route properties")?;
        self.expect_word("delivery")?;
        self.expect_symbol(TokenKind::Equal, "`=` after `delivery`")?;
        let delivery = self.word("delivery value")?;
        self.expect_symbol(TokenKind::Semicolon, "`;` after delivery")?;
        self.expect_symbol(TokenKind::RightBrace, "`}` after route binding")?;
        Ok(RouteBinding { message, delivery })
    }

    fn parse_rpc(&mut self) -> Result<RpcBinding, ProfileParseError> {
        let name = self.word("RPC service name")?;
        self.expect_symbol(TokenKind::LeftBrace, "`{` before RPC properties")?;
        let mut request = None;
        let mut response = None;
        let mut request_operation_id = None;
        let mut response_operation_id = None;
        let mut response_status = None;
        let mut request_delivery = None;
        let mut response_delivery = None;

        while self.current().kind != TokenKind::RightBrace {
            if self.current().kind == TokenKind::End {
                return Err(self.error_current("expected `}` after RPC binding"));
            }
            let property = self.word("RPC property name")?;
            self.expect_symbol(TokenKind::Equal, "`=` after RPC property")?;
            let value = self.word("RPC property value")?;
            self.expect_symbol(TokenKind::Semicolon, "`;` after RPC property")?;
            let target = match property.value.as_str() {
                "request" => &mut request,
                "response" => &mut response,
                "request_operation_id" => &mut request_operation_id,
                "response_operation_id" => &mut response_operation_id,
                "response_status" => &mut response_status,
                "request_delivery" => &mut request_delivery,
                "response_delivery" => &mut response_delivery,
                _ => {
                    return Err(self.error(
                        property.span,
                        format!("unknown RPC property `{}`", property.value),
                    ));
                }
            };
            if target.replace(value).is_some() {
                return Err(self.error(
                    property.span,
                    format!("duplicate RPC property `{}`", property.value),
                ));
            }
        }
        let closing_span = self.current().span;
        self.advance();

        Ok(RpcBinding {
            name,
            request: required_property(request, "request", closing_span)?,
            response: required_property(response, "response", closing_span)?,
            request_operation_id,
            response_operation_id,
            response_status,
            request_delivery: required_property(
                request_delivery,
                "request_delivery",
                closing_span,
            )?,
            response_delivery: required_property(
                response_delivery,
                "response_delivery",
                closing_span,
            )?,
        })
    }

    fn expect_word(&mut self, expected: &str) -> Result<(), ProfileParseError> {
        let word = self.word(&format!("`{expected}`"))?;
        if word.value == expected {
            Ok(())
        } else {
            Err(self.error(word.span, format!("expected `{expected}`")))
        }
    }

    fn word(&mut self, expected: &str) -> Result<Spanned<String>, ProfileParseError> {
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

    fn positive_u32(&mut self, description: &str) -> Result<Spanned<u32>, ProfileParseError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Number(value) if value != 0 => {
                self.advance();
                Ok(Spanned {
                    value,
                    span: token.span,
                })
            }
            TokenKind::Number(_) => Err(self.error(
                token.span,
                format!("{description} must be in 1..=4294967295"),
            )),
            _ => Err(self.error(token.span, format!("expected {description}"))),
        }
    }

    fn expect_symbol(
        &mut self,
        expected: TokenKind,
        description: &str,
    ) -> Result<(), ProfileParseError> {
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

    fn error_current(&self, message: impl Into<String>) -> ProfileParseError {
        self.error(self.current().span, message)
    }

    fn error(&self, span: Span, message: impl Into<String>) -> ProfileParseError {
        ProfileParseError::new(span, message)
    }
}

fn required_property(
    value: Option<Spanned<String>>,
    name: &str,
    span: Span,
) -> Result<Spanned<String>, ProfileParseError> {
    value.ok_or_else(|| ProfileParseError::new(span, format!("missing RPC property `{name}`")))
}
