//! Wirelink schema parsing and validation.

pub mod ast;
mod lexer;
mod parser;
pub mod semantic;

pub use parser::{ParseError, parse_schema};
pub use semantic::{SemanticErrors, SemanticModel, analyze_schema, check_compatibility};
