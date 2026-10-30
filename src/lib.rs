//! Wirelink schema parsing and validation.

pub mod ast;
pub mod codec_plan;
pub mod codegen;
pub mod endpoint_layout;
pub mod identity;
pub mod kernel_codegen;
pub mod kernel_profile;
mod lexer;
mod loader;
pub mod manifest;
mod parser;
pub mod plan_manifest;
pub mod profile;
pub mod profile_semantic;
pub mod sdk_codegen;
pub mod semantic;
mod template;
mod value_codegen;

pub use codegen::{GeneratedC, generate_c, generate_c_for_profile};
pub use identity::{IDENTITY_ALGORITHM, binding_profile_identity, schema_identity};
pub use kernel_codegen::{GeneratedKernelC, generate_kernel_c};
pub use kernel_profile::{KernelProfile, analyze_kernel_profile, compose_kernel_profiles};
pub use loader::{LoadedSchema, load_schema};
pub use manifest::{
    ARTIFACT_DIGEST_ALGORITHM, CODEGEN_CONTRACT_MAJOR, CODEGEN_CONTRACT_MINOR,
    CODEGEN_MANIFEST_FORMAT, COMPILER_NAME, COMPILER_VERSION, ManifestArtifact,
    generate_codegen_manifest,
};
pub use parser::{ParseError, parse_schema};
pub use profile::{BindingProfile, ProfileParseError, parse_binding_profile};
pub use profile_semantic::{
    BindingProfileModel, ProfileSemanticErrors, analyze_binding_profile, compose_binding_profiles,
};
pub use sdk_codegen::{
    BINDING_API_VERSION, GeneratedSdk, SdkCodegenError, SdkOptions, generate_sdk,
};
pub use semantic::{SemanticErrors, SemanticModel, analyze_schema, check_compatibility};
