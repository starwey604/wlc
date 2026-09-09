// SPDX-License-Identifier: Apache-2.0
//! File-relative, static schema composition. There is one global ID/type namespace.
use crate::{ast::Schema, lexer::tokenize, parse_schema};
use miette::{IntoDiagnostic, NamedSource, Result, WrapErr};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct LoadedSchema {
    pub schema: Schema,
    /// Flattened diagnostic source; file labels and original line breaks are retained.
    pub source: String,
    /// Canonical transitive inputs, including the root, for build dependency tracking.
    pub dependencies: Vec<PathBuf>,
}

/// Imports are relative to their containing file. Shared imports are included once;
/// cycles, missing files and duplicate declarations/reservations are errors.
/// The root's version is the composed contract revision; imported revisions remain
/// independently maintained. Paths and traversal order do not enter wire identities.
pub fn load_schema(path: &Path) -> Result<LoadedSchema> {
    let mut files = BTreeMap::new();
    let version = visit(path, &mut Vec::new(), &mut files)?;
    let dependencies = files.keys().cloned().collect();
    let mut source = format!("version {version};\n");
    for (path, (_, body)) in files {
        source.push_str(&format!("// source: {}\n", path.display()));
        source.push_str(&body);
        source.push('\n');
    }
    let schema = parse_schema(&source).map_err(|error| {
        miette::Report::new(error)
            .with_source_code(NamedSource::new(path.display().to_string(), source.clone()))
    })?;
    Ok(LoadedSchema {
        schema,
        source,
        dependencies,
    })
}

fn visit(
    path: &Path,
    active: &mut Vec<PathBuf>,
    files: &mut BTreeMap<PathBuf, (u32, String)>,
) -> Result<u32> {
    let path = path
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("cannot resolve schema {}", path.display()))?;
    if active.contains(&path) {
        return Err(miette::miette!(
            "schema import cycle: {} -> {}",
            active
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> "),
            path.display()
        ));
    }
    if let Some((version, _)) = files.get(&path) {
        return Ok(*version);
    }
    if active.len() >= 64 {
        return Err(miette::miette!(
            "schema import depth exceeds 64 at {}",
            path.display()
        ));
    }
    let source = fs::read_to_string(&path)
        .into_diagnostic()
        .wrap_err_with(|| format!("cannot read schema {}", path.display()))?;
    let schema = parse_schema(&source).map_err(|error| {
        miette::Report::new(error)
            .with_source_code(NamedSource::new(path.display().to_string(), source.clone()))
    })?;
    active.push(path.clone());
    let mut ranges = Vec::new();
    // Parsing already validated `version N;`; blanks preserve diagnostic lines.
    let tokens = tokenize(&source).expect("parsed schema tokenizes");
    ranges.push(0..tokens[2].span.offset + tokens[2].span.length);
    for import in &schema.imports {
        visit(
            &path
                .parent()
                .expect("canonical file has parent")
                .join(&import.value),
            active,
            files,
        )
        .wrap_err_with(|| format!("imported from {}:{}", path.display(), import.span.line))?;
        ranges.push(import.span.offset..import.span.offset + import.span.length);
    }
    active.pop();
    let mut body = source.into_bytes();
    for range in ranges {
        for byte in &mut body[range] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    files.insert(
        path,
        (
            schema.version.value,
            String::from_utf8(body).expect("blanked UTF-8 source"),
        ),
    );
    Ok(schema.version.value)
}
