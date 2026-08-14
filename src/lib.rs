//! Wirelink schema parsing and validation.

pub mod ast;
mod lexer;
mod parser;

pub use parser::{ParseError, parse_schema};
