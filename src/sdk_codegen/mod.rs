// SPDX-License-Identifier: Apache-2.0
//! Generate an owned C++20 / typed Python synchronous/asynchronous UDP SDK project.
use std::collections::BTreeMap;

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    ManifestArtifact, SemanticModel, generate_codegen_manifest,
    node::{Handlers, Node},
    node_identity,
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
    node: &Node,
    options: &SdkOptions,
) -> Result<GeneratedSdk, SdkCodegenError> {
    let plan = plan::Plan::new(schema, node, options)?;
    let name = &options.name;
    let mut client = node.clone();
    client.name = name.clone();
    client.handlers = Handlers::Static;
    // The host bridge supplies a per-call completion through the explicit API.
    for call in &mut client.calls {
        call.handler = None;
    }
    let artifacts =
        crate::node_codegen::generate_node_c_with_pruning(schema, &[client], name, false)
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
    let manifest = manifest(schema, node, name, &files);
    files.insert("wlc-sdk-manifest.json".into(), manifest);
    Ok(GeneratedSdk { files })
}

fn manifest(
    schema: &SemanticModel,
    node: &Node,
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
    generate_codegen_manifest(name, schema, Some(node_identity(node)), &artifacts)
}
