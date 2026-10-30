// SPDX-License-Identifier: Apache-2.0
//! Generate an owned C++20 / typed Python synchronous/asynchronous UDP SDK project.
use std::collections::BTreeMap;

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    BindingProfileModel, KernelProfile, ManifestArtifact, SemanticModel, binding_profile_identity,
    generate_codegen_manifest, generate_kernel_c,
};

mod cpp;
mod package;
mod plan;
mod python;

/// Host source API revision, independent of the generated-code contract.
pub const BINDING_API_VERSION: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SdkOptions {
    /// Portable lower_snake_case C++ namespace; Python package is <name>_sdk.
    pub name: String,
    /// MAJOR.MINOR.PATCH, optionally followed by .devN.
    pub package_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedSdk {
    /// Sorted project-relative paths. No host paths or timestamps are embedded.
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("SDK generation failed: {0}")]
#[diagnostic(code(wlc::sdk_codegen))]
pub struct SdkCodegenError(pub String);

/// Validate the whole project before emitting anything. Reuses the C wire implementation.
pub fn generate_sdk(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    options: &SdkOptions,
) -> Result<GeneratedSdk, SdkCodegenError> {
    let plan = plan::Plan::new(schema, profile, options)?;
    let name = &options.name;
    let kernel = KernelProfile::from_bindings(profile.clone())
        .map_err(|e| SdkCodegenError(e.to_string()))?;
    let artifacts = generate_kernel_c(schema, &kernel, name)
        .map_err(|e| SdkCodegenError(e.to_string()))?
        .files;
    let mut files = artifacts
        .into_iter()
        .map(|(p, c)| (format!("generated/{p}"), c))
        .collect::<BTreeMap<_, _>>();
    cpp::emit(&plan, &mut files);
    python::emit(&plan, &mut files);
    package::emit(&plan, &mut files);
    files.insert("wlc-sdk-name.txt".into(), format!("{name}\n"));
    let manifest = manifest(schema, profile, name, &files);
    files.insert("wlc-sdk-manifest.json".into(), manifest);
    Ok(GeneratedSdk { files })
}

fn manifest(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    name: &str,
    files: &BTreeMap<String, String>,
) -> String {
    let artifacts = files
        .iter()
        .map(|(path, contents)| ManifestArtifact {
            path,
            contents: contents.as_bytes(),
        })
        .collect::<Vec<_>>();
    generate_codegen_manifest(
        name,
        schema,
        Some(binding_profile_identity(profile)),
        &artifacts,
    )
}
