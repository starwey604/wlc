//! Wirelink schema parsing and validation.

pub mod ast;
pub mod codec_plan;
pub mod codegen;
pub mod identity;
mod lexer;
mod loader;
pub mod manifest;
pub mod node;
pub mod node_codegen;
mod parser;
pub mod plan_manifest;
pub mod sdk_codegen;
pub mod semantic;
mod template;

pub use codegen::{GeneratedC, generate_c, generate_c_for_nodes};
pub use identity::{IDENTITY_ALGORITHM, node_identity, schema_identity};
pub use loader::{LoadedSchema, load_schema};
pub use manifest::{
    ARTIFACT_DIGEST_ALGORITHM, CODEGEN_CONTRACT_MAJOR, CODEGEN_CONTRACT_MINOR,
    CODEGEN_MANIFEST_FORMAT, COMPILER_NAME, COMPILER_VERSION, ManifestArtifact,
    generate_codegen_manifest,
};
pub use node::{Node, NodeError, analyze_nodes};
pub use node_codegen::{GeneratedNodeC, generate_node_c};
pub use parser::{ParseError, parse_schema};
pub use sdk_codegen::{
    BINDING_API_VERSION, GeneratedSdk, SdkCodegenError, SdkOptions, generate_sdk,
};
pub use semantic::{SemanticErrors, SemanticModel, analyze_schema, check_compatibility};
