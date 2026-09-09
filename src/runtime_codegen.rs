//! Deterministic C runtime generation for an optional binding profile.
use crate::{
    codegen::{CModel, c_identifier, upper_snake},
    profile_semantic::BindingProfileModel,
    semantic::SemanticModel,
};
use miette::Diagnostic;
use thiserror::Error;

mod assembly;
mod dispatch;
mod header;
mod pump;
mod retained;
mod rpc;
mod validation;

use dispatch::emit_source;
use header::emit_header;
use validation::{validate_profile_model, validate_runtime_names};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedRuntimeC {
    pub header: String,
    pub endpoint_header: String,
    pub advanced_header: String,
    pub source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C runtime generation failed: {0}")]
#[diagnostic(code(wlc::runtime_codegen))]
pub struct RuntimeCodegenError(pub String);

/// Emit an optional application runtime translation unit for a resolved
/// binding profile. The ordinary codec and binding artifacts remain separate
/// and byte-for-byte independent of this function.
pub fn generate_runtime_c(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    module_name: &str,
) -> Result<GeneratedRuntimeC, RuntimeCodegenError> {
    generate_runtime_c_named(schema, profile, module_name, module_name)
}

/// Emit a runtime whose public symbols use `runtime_name` while codec and
/// typed-send references continue to use `codec_module_name`. This lets one
/// schema target back multiple asymmetric profile runtimes without duplicate
/// codec symbols.
pub fn generate_runtime_c_named(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    codec_module_name: &str,
    runtime_name: &str,
) -> Result<GeneratedRuntimeC, RuntimeCodegenError> {
    // Share validation/bounds, without rendering and discarding codec artifacts.
    let facts =
        CModel::new(schema, codec_module_name).map_err(|error| RuntimeCodegenError(error.0))?;
    let codec_module = facts.module;
    let runtime = c_identifier(runtime_name);
    if runtime.is_empty() {
        return Err(RuntimeCodegenError(
            "runtime name has no C identifier characters".to_owned(),
        ));
    }
    validate_profile_model(schema, profile)?;
    validate_runtime_names(schema, profile, &runtime)?;

    Ok(GeneratedRuntimeC {
        header: emit_header(schema, profile, &codec_module, &runtime, &facts.maxima),
        endpoint_header: format!(
            "/* SPDX-License-Identifier: Apache-2.0 */\n/* Ordinary application entry: owned business values and default endpoint.\n * runtime.h is a transitive layout dependency, not the ordinary API contract.\n * private_state, token/call/inspect/release and runtime assembly are advanced. */\n#ifndef {0}_ENDPOINT_H\n#define {0}_ENDPOINT_H\n#include \"{codec_module}_values.h\"\n#include \"{runtime}_runtime.h\"\n#endif\n",
            upper_snake(&runtime)
        ),
        source: emit_source(&facts.maxima, profile, &codec_module, &runtime),
        advanced_header: crate::endpoint_codegen::advanced(profile, &runtime),
    })
}
