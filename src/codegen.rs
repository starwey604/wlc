// SPDX-License-Identifier: Apache-2.0
//! Static, allocation-free C11 codec generation.
use std::collections::HashMap;

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    codec_plan::{CodecPlan, CodecProfile},
    node::Node,
    semantic::{ResolvedType, SemanticModel, Symbol},
};

mod descriptor;
mod engine;
mod fixed;
mod flat;
mod header;
pub(crate) mod local;
mod names;
mod packed;
pub(crate) mod plan;
mod positional;
mod positional_fixed;
mod symbols;
mod wire;

pub(crate) use descriptor::field_descriptor_data;
use engine::emit_source;
pub use names::c_identifier;
pub use names::c_identifier as c_module_name;
pub(crate) use names::{c_type, is_c_keyword, type_name, upper_snake};
use symbols::Symbols;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MessageDirections {
    pub(crate) encode: bool,
    pub(crate) decode: bool,
}

impl Default for MessageDirections {
    fn default() -> Self {
        Self {
            encode: true,
            decode: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedC {
    pub header: String,
    pub source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C code generation failed: {0}")]
#[diagnostic(code(wlc::codegen))]
pub struct CodegenError(pub String);

/// Generate a C11 codec with Schema-v1 defaults across all messages.
pub fn generate_c(model: &SemanticModel, module: &str) -> Result<GeneratedC, CodegenError> {
    let codecs = CodecPlan::new(model, CodecProfile::SchemaV1)
        .map_err(|error| CodegenError(error.to_string()))?;
    emit_c(model, module, &codecs, &all_directions(model))
}

/// Generate codecs for selected nodes, optionally pruning unused directions.
pub fn generate_c_for_nodes(
    model: &SemanticModel,
    nodes: &[Node],
    module: &str,
    prune_routes: bool,
) -> Result<GeneratedC, CodegenError> {
    let mut selected = HashMap::new();
    for node in nodes {
        for route in node.send_routes.iter().chain(&node.recv_routes) {
            if let Some(existing) = selected.insert(route.message_id, route.codec)
                && existing != route.codec
            {
                return Err(CodegenError(format!(
                    "conflicting codec selection for message `{}`",
                    route.message_name
                )));
            }
        }
        for rpc in node.calls.iter().chain(&node.serves) {
            for id in [rpc.request_id, rpc.response_id] {
                if let Some(existing) = selected.insert(id, CodecProfile::SchemaV1)
                    && existing != CodecProfile::SchemaV1
                {
                    return Err(CodegenError(
                        "RPC conflicts with ordinary positional codec".into(),
                    ));
                }
            }
        }
    }
    let codecs = CodecPlan::with_overrides(model, CodecProfile::SchemaV1, selected)
        .map_err(|error| CodegenError(error.to_string()))?;
    let directions = if prune_routes {
        route_directions(model, nodes)
    } else {
        all_directions(model)
    };
    emit_c(model, module, &codecs, &directions)
}

fn all_directions(model: &SemanticModel) -> HashMap<u16, MessageDirections> {
    model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(m) => Some((m.id, MessageDirections::default())),
            Symbol::Enum(_) => None,
        })
        .collect()
}

fn route_directions(model: &SemanticModel, nodes: &[Node]) -> HashMap<u16, MessageDirections> {
    let mut dirs: HashMap<u16, MessageDirections> = model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(m) => Some((
                m.id,
                MessageDirections {
                    encode: false,
                    decode: false,
                },
            )),
            Symbol::Enum(_) => None,
        })
        .collect();

    for node in nodes {
        for route in &node.send_routes {
            if let Some(entry) = dirs.get_mut(&route.message_id) {
                entry.encode = true;
            }
        }
        for route in &node.recv_routes {
            if let Some(entry) = dirs.get_mut(&route.message_id) {
                entry.decode = true;
            }
        }
        for rpc in &node.calls {
            if let Some(entry) = dirs.get_mut(&rpc.request_id) {
                entry.encode = true;
            }
            if let Some(entry) = dirs.get_mut(&rpc.response_id) {
                entry.decode = true;
            }
        }
        for rpc in &node.serves {
            if let Some(entry) = dirs.get_mut(&rpc.request_id) {
                entry.decode = true;
            }
            if let Some(entry) = dirs.get_mut(&rpc.response_id) {
                entry.encode = true;
            }
        }
    }

    // Propagate parent direction needs transitively down to nested child messages.
    if let Ok(ordered) = plan::CModel::new(model, "_prop") {
        for message in ordered.messages.iter().rev() {
            let parent_dir = dirs.get(&message.id).copied().unwrap_or_default();
            if parent_dir.encode || parent_dir.decode {
                for field in &message.fields {
                    if let ResolvedType::Message { id, .. } = field.ty
                        && let Some(child_dir) = dirs.get_mut(&id)
                    {
                        child_dir.encode |= parent_dir.encode;
                        child_dir.decode |= parent_dir.decode;
                    }
                }
            }
        }
    }
    dirs
}

fn emit_c(
    schema: &SemanticModel,
    module: &str,
    codecs: &CodecPlan<'_>,
    directions: &HashMap<u16, MessageDirections>,
) -> Result<GeneratedC, CodegenError> {
    crate::node_codegen::check_names(schema, &[]).map_err(|e| CodegenError(e.to_string()))?;
    let model = plan::CModel::new(schema, module)?;
    let module = model.module;
    let messages = model.messages;

    let mut uses_float32 = false;
    let mut uses_float64 = false;
    for symbol in &schema.declarations {
        if let Symbol::Message(message) = symbol {
            for field in &message.fields {
                uses_float32 |= matches!(field.ty, ResolvedType::Float32);
                uses_float64 |= matches!(field.ty, ResolvedType::Float64);
            }
        }
    }

    let guard = format!("WIRELINK_GENERATED_{}_H", upper_snake(&module));
    let mut header = format!(
        "/* SPDX-License-Identifier: Apache-2.0 */\n#ifndef {guard}\n#define {guard}\n\n#if !defined(__cplusplus)\n#include <stdbool.h>\n#endif\n#include <stddef.h>\n#include <stdint.h>\n#include <wirelink/codec.h>\n"
    );
    if uses_float32 || uses_float64 {
        header.push_str("#include <float.h>\n\n#if defined(__cplusplus)\n");
        if uses_float32 {
            header.push_str("static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 && FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125, \"WLC float32 requires IEEE-754 binary32\");\n");
        }
        if uses_float64 {
            header.push_str("static_assert(sizeof(double) == 8 && FLT_RADIX == 2 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024 && DBL_MIN_EXP == -1021, \"WLC float64 requires IEEE-754 binary64\");\n");
        }
        header.push_str("#else\n");
        if uses_float32 {
            header.push_str("_Static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 && FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125, \"WLC float32 requires IEEE-754 binary32\");\n");
        }
        if uses_float64 {
            header.push_str("_Static_assert(sizeof(double) == 8 && FLT_RADIX == 2 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024 && DBL_MIN_EXP == -1021, \"WLC float64 requires IEEE-754 binary64\");\n");
        }
        header.push_str("#endif\n");
    }
    header.push_str("\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");

    for symbol in &schema.declarations {
        if let Symbol::Enum(enumeration) = symbol {
            let name = type_name(&enumeration.name);
            header.push_str(&format!("typedef int32_t {name}_t;\n"));
            for value in &enumeration.values {
                header.push_str(&format!(
                    "#define {} INT32_C({})\n",
                    upper_snake(&value.name),
                    value.number
                ));
            }
            header.push('\n');
        }
    }

    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!("typedef struct {name} {name}_t;\n"));
    }
    if !messages.is_empty() {
        header.push('\n');
    }

    for message in &messages {
        header::emit_message_definition(&mut header, message);
    }

    for message in &messages {
        let name = type_name(&message.name);
        let macro_name = upper_snake(&message.name);
        let plan = &codecs.messages[&message.id];
        let dir = directions.get(&message.id).copied().unwrap_or_default();
        header.push_str(&format!(
            "#define {macro_name}_MESSAGE_ID {}U\n#define {macro_name}_CODEC_PROFILE {}U\n#define {macro_name}_CODEC_FORMAT \"{}\"\n",
            message.id,
            plan.profile.code(),
            plan.profile.name()
        ));
        match plan.max_encoded_size {
            Some(maximum) => header.push_str(&format!(
                "#define {macro_name}_HAS_MAX_ENCODED_SIZE 1\n#define {macro_name}_MAX_ENCODED_SIZE UINT64_C({maximum})\n"
            )),
            None => header.push_str(&format!("#define {macro_name}_HAS_MAX_ENCODED_SIZE 0\n")),
        }
        header.push_str(&format!("void {name}_clear({name}_t *value);\n"));
        if dir.encode {
            header.push_str(&format!(
                "size_t {name}_encoded_size(const {name}_t *value);\nwl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length);\n"
            ));
        }
        if dir.decode {
            header.push_str(&format!(
                "wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out);\n"
            ));
        }
        header.push('\n');
    }

    header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");

    let symbols = Symbols::public();
    let source = emit_source(&module, &messages, codecs, directions, &symbols);

    Ok(GeneratedC { header, source })
}
