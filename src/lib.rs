//! Wirelink schema parsing and validation.

pub mod ast;
pub mod codegen;
pub mod identity;
mod lexer;
mod parser;
pub mod profile;
pub mod profile_semantic;
pub mod runtime_codegen;
pub mod semantic;

pub use codegen::{GeneratedC, generate_c};
pub use identity::{IDENTITY_ALGORITHM, binding_profile_identity, schema_identity};
pub use parser::{ParseError, parse_schema};
pub use profile::{BindingProfile, ProfileParseError, parse_binding_profile};
pub use profile_semantic::{BindingProfileModel, ProfileSemanticErrors, analyze_binding_profile};
pub use runtime_codegen::{GeneratedRuntimeC, RuntimeCodegenError, generate_runtime_c};
pub use semantic::{SemanticErrors, SemanticModel, analyze_schema, check_compatibility};
