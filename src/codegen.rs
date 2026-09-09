//! Deterministic C declaration generation for schema v1.

use miette::Diagnostic;
use thiserror::Error;

use crate::semantic::{SemanticModel, Symbol};

mod bindings;
mod bounds;
mod descriptor;
mod engine;
mod header;
mod names;
mod packed;
mod plan;
mod wire;

use bindings::{emit_bindings_header, emit_bindings_source};
pub(crate) use descriptor::field_descriptor_data;
use engine::emit_source;
use header::emit_message_definition;
use names::ieee_float_usage;
pub(crate) use names::{c_identifier, c_type, type_name, upper_snake};
pub(crate) use plan::CModel;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedC {
    pub header: String,
    /// Standalone, pointer-free business types and codec entry points.
    pub values_header: String,
    pub source: String,
    /// Optional Wirelink-core bindings, kept in a separate translation unit so
    /// codec-only users do not acquire link-core symbol dependencies.
    pub bindings_header: String,
    pub bindings_source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C generation failed: {0}")]
#[diagnostic(code(wlc::codegen))]
pub struct CodegenError(pub String);

/// Emits standalone codec and Wirelink-binding translation units for one schema.
/// `module_name` controls output includes, symbols, and header guards.
pub fn generate_c(model: &SemanticModel, module_name: &str) -> Result<GeneratedC, CodegenError> {
    let plan = CModel::new(model, module_name)?;
    let module = plan.module;
    let guard = format!("WIRELINK_GENERATED_{}_H", upper_snake(&module));
    let values_guard = format!("{guard}_VALUES");
    let mut values_header = format!(
        "#ifndef {values_guard}\n#define {values_guard}\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <wirelink/codec.h>\n"
    );
    let (uses_float32, uses_float64) = ieee_float_usage(model);
    if uses_float32 || uses_float64 {
        values_header.push_str("#include <float.h>\n\n#if defined(__cplusplus)\n");
        if uses_float32 {
            values_header.push_str("static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 && FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125, \"WLC float32 requires IEEE-754 binary32\");\n");
        }
        if uses_float64 {
            values_header.push_str("static_assert(sizeof(double) == 8 && FLT_RADIX == 2 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024 && DBL_MIN_EXP == -1021, \"WLC float64 requires IEEE-754 binary64\");\n");
        }
        values_header.push_str("#else\n");
        if uses_float32 {
            values_header.push_str("_Static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 && FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125, \"WLC float32 requires IEEE-754 binary32\");\n");
        }
        if uses_float64 {
            values_header.push_str("_Static_assert(sizeof(double) == 8 && FLT_RADIX == 2 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024 && DBL_MIN_EXP == -1021, \"WLC float64 requires IEEE-754 binary64\");\n");
        }
        values_header.push_str("#endif\n");
    }
    values_header.push_str("\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    let mut header = format!(
        "#ifndef {guard}\n#define {guard}\n\n/* Advanced borrowed codec and explicit value/view conversions. */\n#include \"{module}_values.h\"\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    let messages = plan.messages;
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!("typedef struct {name} {name}_t;\n"));
    }
    if !messages.is_empty() {
        header.push('\n');
    }
    let static_max_encoded_sizes = plan.maxima;
    for symbol in &model.declarations {
        if let Symbol::Enum(enumeration) = symbol {
            let name = type_name(&enumeration.name);
            values_header.push_str(&format!("typedef int32_t {name}_t;\n"));
            for value in &enumeration.values {
                values_header.push_str(&format!(
                    "#define {} INT32_C({})\n",
                    upper_snake(&value.name),
                    value.number
                ));
            }
            values_header.push('\n');
        }
    }
    for message in &messages {
        emit_message_definition(&mut header, message);
        header.push('\n');
    }
    crate::value_codegen::header(&mut values_header, &messages, &static_max_encoded_sizes);
    for message in &messages {
        let name = type_name(&message.name);
        let macro_name = upper_snake(&message.name);
        values_header.push_str(&format!(
            "#define {macro_name}_MESSAGE_ID {}U\n",
            message.id,
        ));
        match static_max_encoded_sizes.get(&message.id).copied().flatten() {
            Some(maximum) => values_header.push_str(&format!(
                "#define {macro_name}_HAS_MAX_ENCODED_SIZE 1\n#define {macro_name}_MAX_ENCODED_SIZE UINT64_C({maximum})\n"
            )),
            None => values_header.push_str(&format!(
                "#define {macro_name}_HAS_MAX_ENCODED_SIZE 0\n"
            )),
        }
        header.push_str(&format!("void {name}_clear({name}_t *value);\n"));
        header.push_str(&format!(
            "size_t {name}_encoded_size(const {name}_t *value);\n"
        ));
        header.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length);\n"));
        header.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out);\n\n"));
        if static_max_encoded_sizes
            .get(&message.id)
            .copied()
            .flatten()
            .is_some()
        {
            header.push_str(&format!("/* Advanced conversions: views borrow value/input storage. Inputs and outputs\n * must not overlap; failures leave output unchanged. */\nwl_codec_status_t {name}_value_from_view(const {name}_t *view, {name}_value_t *out);\nwl_codec_status_t {name}_value_to_view(const {name}_value_t *value, {name}_t *out);\n\n"));
        }
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    values_header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    let mut source = emit_source(&module, &messages);
    crate::value_codegen::source(&mut source, &messages, &static_max_encoded_sizes);
    let bindings_header = emit_bindings_header(&module, &guard, &messages);
    let bindings_source = emit_bindings_source(&module, &messages);
    Ok(GeneratedC {
        header,
        values_header,
        source,
        bindings_header,
        bindings_source,
    })
}
