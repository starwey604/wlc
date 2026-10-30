//! Deterministic provenance manifests for generated artifacts.

use std::fmt::Write;

use crate::{
    identity::schema_identity,
    semantic::{ResolvedType, SemanticModel, Symbol},
};

/// Generated-code contract major revision; mirrors the compiler major version.
pub const CODEGEN_CONTRACT_MAJOR: u16 = version_component(COMPILER_VERSION, 0);
/// Generated-code contract minor revision; mirrors the compiler minor version.
pub const CODEGEN_CONTRACT_MINOR: u16 = version_component(COMPILER_VERSION, 1);
/// Stable JSON manifest format identifier.
pub const CODEGEN_MANIFEST_FORMAT: &str = "wirelink-codegen-manifest-v1";
/// Diagnostic, non-cryptographic artifact digest algorithm.
pub const ARTIFACT_DIGEST_ALGORITHM: &str = "fnv1a64-domain-bytes-v1";
/// Compiler name embedded in generated provenance.
pub const COMPILER_NAME: &str = "wlc";
/// Cargo package version embedded in generated provenance.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read one dotted numeric component of a `MAJOR.MINOR.PATCH` version.
const fn version_component(version: &str, component: usize) -> u16 {
    let bytes = version.as_bytes();
    let mut index = 0;
    let mut current = 0;
    let mut value: u16 = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        if byte == b'.' {
            if current == component {
                break;
            }
            current += 1;
            value = 0;
            continue;
        }
        if byte < b'0' || byte > b'9' {
            break;
        }
        if current == component {
            value = value * 10 + (byte - b'0') as u16;
        }
    }
    value
}

const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;
const ARTIFACT_DIGEST_DOMAIN: &[u8] = b"wlc.generated-artifact.v1";

/// One generated artifact included in a provenance manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestArtifact<'a> {
    pub path: &'a str,
    pub contents: &'a [u8],
}

/// Emit stable JSON without timestamps, source paths, host details, or map
/// iteration order. Artifact paths are sorted lexically before emission.
pub fn generate_codegen_manifest(
    module: &str,
    schema: &SemanticModel,
    binding_profile_identity: Option<u64>,
    artifacts: &[ManifestArtifact<'_>],
) -> String {
    let schema_identity = schema_identity(schema);
    let mut artifacts = artifacts.iter().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        left.path
            .cmp(right.path)
            .then_with(|| left.contents.cmp(right.contents))
    });

    let mut output = String::new();
    output.push_str("{\n  \"format\": ");
    push_json_string(&mut output, CODEGEN_MANIFEST_FORMAT);
    output.push_str(",\n  \"compiler\": {\n    \"name\": ");
    push_json_string(&mut output, COMPILER_NAME);
    output.push_str(",\n    \"version\": ");
    push_json_string(&mut output, COMPILER_VERSION);
    writeln!(
        output,
        ",\n    \"codegen_contract\": {{\"major\": {CODEGEN_CONTRACT_MAJOR}, \"minor\": {CODEGEN_CONTRACT_MINOR}}}\n  }},"
    )
    .unwrap();
    output.push_str("  \"module\": ");
    push_json_string(&mut output, module);
    output.push_str(",\n  \"identity\": {\n    \"algorithm\": ");
    push_json_string(&mut output, crate::IDENTITY_ALGORITHM);
    write!(
        output,
        ",\n    \"schema\": \"0x{schema_identity:016x}\",\n    \"binding_profile\": "
    )
    .unwrap();
    match binding_profile_identity {
        Some(identity) => write!(output, "\"0x{identity:016x}\"").unwrap(),
        None => output.push_str("null"),
    }
    output.push_str("\n  },\n");
    let managed_wire = artifacts.iter().find_map(|artifact| {
        let text = std::str::from_utf8(artifact.contents).ok()?;
        text.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != "#define" || !words.next()?.ends_with("_RPC_MANAGED_WIRE_FORMAT") {
                return None;
            }
            words.next()?.strip_prefix('"')?.strip_suffix('"')
        })
    });
    if let Some(wire) = managed_wire {
        output.push_str("  \"managed_rpc_wire_format\": ");
        push_json_string(&mut output, wire);
        output.push_str(",\n");
    }
    let mut codecs = std::collections::BTreeMap::new();
    for artifact in &artifacts {
        if let Ok(text) = std::str::from_utf8(artifact.contents) {
            for line in text.lines() {
                let mut words = line.split_whitespace();
                if words.next() != Some("#define") {
                    continue;
                }
                if let (Some(name), Some(format)) = (words.next(), words.next())
                    && (name.ends_with("_CODEC_FORMAT") || name.ends_with("_RPC_BODY_FORMAT"))
                {
                    codecs.insert(name, format.trim_matches('"'));
                }
            }
        }
    }
    output.push_str("  \"codec_formats\": {");
    for (index, (symbol, format)) in codecs.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str("\n    ");
        push_json_string(&mut output, symbol);
        output.push_str(": ");
        push_json_string(&mut output, format);
    }
    output.push_str("\n  },\n");
    output.push_str("  \"bounded_fields\": [\n");
    let bounded_fields = schema
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some(message.fields.iter().filter_map(move |field| {
                let max_length = field.max_length?;
                let kind = match field.ty {
                    ResolvedType::Bytes => "bytes",
                    ResolvedType::String => "string",
                    _ => unreachable!("only string and bytes fields may carry a bound"),
                };
                Some((message, field, kind, max_length))
            })),
            Symbol::Enum(_) => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    for (index, (message, field, kind, max_length)) in bounded_fields.iter().enumerate() {
        output.push_str("    {\"message\": ");
        push_json_string(&mut output, &message.name);
        write!(output, ", \"message_id\": {}, \"field\": ", message.id).unwrap();
        push_json_string(&mut output, &field.name);
        writeln!(
            output,
            ", \"field_number\": {}, \"kind\": \"{kind}\", \"max_length\": {max_length}}}{}",
            field.number,
            if index + 1 == bounded_fields.len() {
                ""
            } else {
                ","
            }
        )
        .unwrap();
    }
    output.push_str("  ],\n  \"artifact_digest_algorithm\": ");
    push_json_string(&mut output, ARTIFACT_DIGEST_ALGORITHM);
    output.push_str(",\n  \"artifacts\": [\n");
    for (index, artifact) in artifacts.iter().enumerate() {
        output.push_str("    {\"path\": ");
        push_json_string(&mut output, artifact.path);
        writeln!(
            output,
            ", \"size\": {}, \"digest\": \"0x{:016x}\"}}{}",
            artifact.contents.len(),
            artifact_digest(artifact.contents),
            if index + 1 == artifacts.len() {
                ""
            } else {
                ","
            }
        )
        .unwrap();
    }
    output.push_str("  ]\n}\n");
    output
}

fn artifact_digest(contents: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET;
    for byte in ARTIFACT_DIGEST_DOMAIN
        .iter()
        .copied()
        .chain([0xff])
        .chain(contents.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

pub(crate) fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0000}'..='\u{001f}' => write!(output, "\\u{:04x}", character as u32).unwrap(),
            _ => output.push(character),
        }
    }
    output.push('"');
}
