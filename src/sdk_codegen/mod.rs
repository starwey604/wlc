// SPDX-License-Identifier: Apache-2.0
//! Generate an owned C++20 / typed Python synchronous/asynchronous UDP SDK project.
use std::collections::BTreeMap;

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    BindingProfileModel, ManifestArtifact, SemanticModel, binding_profile_identity, generate_c,
    generate_codegen_manifest, generate_runtime_c,
};

mod cpp;
mod package;
mod plan;
mod python;

/// Host source API revision, independent of the generated C layout ABI.
pub const BINDING_API_VERSION: u32 = 2;

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
    let c = generate_c(schema, name).map_err(|e| SdkCodegenError(e.to_string()))?;
    let runtime =
        generate_runtime_c(schema, profile, name).map_err(|e| SdkCodegenError(e.to_string()))?;
    let mut artifacts = BTreeMap::from([
        (format!("{name}.h"), c.header),
        (format!("{name}.c"), c.source),
        (format!("{name}_values.h"), c.values_header),
        (format!("{name}_bindings.h"), c.bindings_header),
        (format!("{name}_bindings.c"), c.bindings_source),
        (format!("{name}_runtime.h"), runtime.header),
        (format!("{name}_runtime.c"), runtime.source),
        (format!("{name}_endpoint.h"), runtime.endpoint_header),
        (format!("{name}_advanced.h"), runtime.advanced_header),
    ]);
    let c_manifest = manifest(schema, profile, name, &artifacts);
    artifacts.insert(format!("{name}_manifest.json"), c_manifest);
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
