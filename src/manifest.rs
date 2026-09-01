//! Deterministic provenance manifests for generated artifacts.

use std::fmt::Write;

/// Semantic revision of generated C APIs and layouts.
pub const CODEGEN_ABI_VERSION: u32 = 3;
/// Stable JSON manifest format identifier.
pub const CODEGEN_MANIFEST_FORMAT: &str = "wirelink-codegen-manifest-v1";
/// Diagnostic, non-cryptographic artifact digest algorithm.
pub const ARTIFACT_DIGEST_ALGORITHM: &str = "fnv1a64-domain-bytes-v1";
/// Compiler name embedded in generated provenance.
pub const COMPILER_NAME: &str = "wlc";
/// Cargo package version embedded in generated provenance.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    schema_identity: u64,
    binding_profile_identity: Option<u64>,
    artifacts: &[ManifestArtifact<'_>],
) -> String {
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
        ",\n    \"codegen_abi\": {CODEGEN_ABI_VERSION}\n  }},"
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
    output.push_str("\n  },\n  \"artifact_digest_algorithm\": ");
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

fn push_json_string(output: &mut String, value: &str) {
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
