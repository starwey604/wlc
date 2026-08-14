use crate::ast::Span;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Identifier(String),
    Number(i64),
    String(String),
    Version,
    Message,
    Enum,
    Optional,
    Repeated,
    Default,
    True,
    False,
    Equal,
    Semicolon,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    End,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LexError {
    pub span: Span,
    pub message: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        source,
        offset: 0,
        line: 1,
        column: 1,
    };
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_end = token.kind == TokenKind::End;
        tokens.push(token);
        if is_end {
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
    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_ignored()?;
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
            '[' => {
                self.advance();
                TokenKind::LeftBracket
            }
            ']' => {
                self.advance();
                TokenKind::RightBracket
            }
            '"' => TokenKind::String(self.read_string(span)?),
            '-' | '0'..='9' => TokenKind::Number(self.read_number(span)?),
            '_' | 'a'..='z' | 'A'..='Z' => self.read_identifier(),
            _ => return Err(self.error(span, format!("unexpected character `{character}`"))),
        };
        Ok(Token { kind, span })
    }

    fn skip_ignored(&mut self) -> Result<(), LexError> {
        loop {
            while matches!(self.peek(), Some(character) if character.is_whitespace()) {
                self.advance();
            }
            if !self.starts_with("//") {
                return Ok(());
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
        match &self.source[start..self.offset] {
            "version" => TokenKind::Version,
            "message" => TokenKind::Message,
            "enum" => TokenKind::Enum,
            "optional" => TokenKind::Optional,
            "repeated" => TokenKind::Repeated,
            "default" => TokenKind::Default,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            name => TokenKind::Identifier(name.to_owned()),
        }
    }

    fn read_number(&mut self, span: Span) -> Result<i64, LexError> {
        let start = self.offset;
        if self.peek() == Some('-') {
            self.advance();
        }
        let digits_start = self.offset;
        while matches!(self.peek(), Some('0'..='9')) {
            self.advance();
        }
        if digits_start == self.offset {
            return Err(self.error(span, "expected digits after `-`"));
        }
        self.source[start..self.offset]
            .parse()
            .map_err(|_| self.error(span, "integer literal is out of range"))
    }

    fn read_string(&mut self, span: Span) -> Result<String, LexError> {
        self.advance();
        let mut value = String::new();
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(span, "unterminated string literal"));
            };
            match character {
                '"' => {
                    self.advance();
                    return Ok(value);
                }
                '\n' | '\r' => {
                    return Err(self.error(span, "string literals cannot contain a newline"));
                }
                '\\' => {
                    self.advance();
                    let escape_span = self.span();
                    let Some(escaped) = self.peek() else {
                        return Err(self.error(escape_span, "unterminated escape sequence"));
                    };
                    match escaped {
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        'n' => value.push('\n'),
                        't' => value.push('\t'),
                        _ => {
                            return Err(self
                                .error(escape_span, format!("unsupported escape `\\{escaped}`")));
                        }
                    }
                    self.advance();
                }
                _ => {
                    value.push(character);
                    self.advance();
                }
            }
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        self.source[self.offset..].starts_with(text)
    }
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn span(&self) -> Span {
        Span {
            line: self.line,
            column: self.column,
        }
    }
    fn error(&self, span: Span, message: impl Into<String>) -> LexError {
        LexError {
            span,
            message: message.into(),
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
