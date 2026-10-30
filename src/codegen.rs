//! Deterministic C declaration generation for schema v1.

use miette::Diagnostic;
use thiserror::Error;

use crate::semantic::{SemanticModel, Symbol};

mod bindings;
mod descriptor;
mod engine;
mod features;
mod fixed;
mod header;
mod names;
mod owned;
mod packed;
mod plan;
pub(crate) mod positional;
mod positional_fixed;
mod symbols;
pub(crate) use symbols::Symbols;
mod wire;

use bindings::{emit_bindings_header, emit_bindings_source};
pub(crate) use descriptor::{field_descriptor_data, tagged_dependencies, tagged_name};
use engine::emit_source;
use header::emit_message_definition;
use names::ieee_float_usage;
pub(crate) use names::{c_identifier, c_type, type_name, upper_snake};
pub(crate) use owned::{owned_decode_source, owned_descriptor, scalar_value_decode};
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
    generate_c_with_fingerprints(model, module_name, None, None)
}

/// Matched profile generation omits canonical fingerprint code for light methods.
/// Standalone codecs retain their generator-private canonical helpers.
pub fn generate_c_for_profile(
    model: &SemanticModel,
    profile: &crate::BindingProfileModel,
    module_name: &str,
) -> Result<GeneratedC, CodegenError> {
    use crate::profile_semantic::RpcGuarantee;
    if profile
        .rpc_services
        .iter()
        .all(|s| s.guarantee == RpcGuarantee::Replay)
        && profile
            .send_routes
            .iter()
            .chain(&profile.direct_routes)
            .all(|r| r.codec == crate::codec_plan::CodecProfile::SchemaV1)
        && profile
            .retained_routes
            .iter()
            .all(|r| r.codec == crate::codec_plan::CodecProfile::SchemaV1)
    {
        return generate_c(model, module_name);
    }
    let requests = profile
        .rpc_services
        .iter()
        .filter(|s| profile.has_rpc_server() && s.guarantee == RpcGuarantee::Replay)
        .map(|s| s.request_id)
        .collect();
    generate_c_with_fingerprints(model, module_name, Some(&requests), Some(profile))
}

pub(crate) fn generate_c_with_fingerprints(
    model: &SemanticModel,
    module_name: &str,
    fingerprints: Option<&std::collections::BTreeSet<u16>>,
    profile: Option<&crate::BindingProfileModel>,
) -> Result<GeneratedC, CodegenError> {
    let mut plan = CModel::new(model, module_name)?;
    let mut codecs =
        crate::codec_plan::CodecPlan::new(model, crate::codec_plan::CodecProfile::SchemaV1)
            .map_err(|e| CodegenError(e.to_string()))?;
    if let Some(profile) = profile {
        for (id, codec) in profile
            .send_routes
            .iter()
            .chain(&profile.direct_routes)
            .map(|r| (r.message_id, r.codec))
            .chain(
                profile
                    .retained_routes
                    .iter()
                    .map(|r| (r.message_id, r.codec)),
            )
        {
            // A route selects only its top-level codec. Nested messages retain
            // the enclosing tagged format, independently of their own routes.
            let mut selected = crate::codec_plan::CodecPlan::selected(model, codec, [id])
                .map_err(|e| CodegenError(e.to_string()))?;
            codecs
                .messages
                .insert(id, selected.messages.remove(&id).unwrap());
        }
    }
    plan.maxima = codecs
        .messages
        .iter()
        .map(|(&id, p)| (id, p.max_encoded_size))
        .collect();
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
            "#define {macro_name}_CODEC_PROFILE {}U\n#define {macro_name}_CODEC_FORMAT \"{}\"\n",
            codecs.messages[&message.id].profile.code(),
            codecs.messages[&message.id].profile.name()
        ));
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
    let symbols = Symbols::public(&module);
    let mut source = emit_source(&messages, &codecs, fingerprints, &symbols);
    crate::value_codegen::source(
        &mut source,
        &messages,
        &static_max_encoded_sizes,
        &codecs,
        &symbols,
    );
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

/// Render private business codecs directly from field plans and a symbol policy.
/// The semantic schema and public business types are borrowed unchanged.
pub(crate) fn private_source(
    model: &SemanticModel,
    module: &str,
    profile: &crate::BindingProfileModel,
) -> String {
    use crate::codec_plan::{CodecPlan, CodecProfile};
    use std::fmt::Write;
    let c = CModel::new(model, module).expect("validated C model");
    let mut codecs =
        CodecPlan::selected(model, CodecProfile::ManagedTagged, []).expect("validated codec plan");
    for service in profile.rpc_services.iter().filter(|s| s.is_managed()) {
        let selected = CodecPlan::selected(
            model,
            service.codec,
            [service.request_id, service.response_id],
        )
        .expect("validated route codec");
        for (id, plan) in selected.messages {
            if id == service.request_id || id == service.response_id {
                codecs.messages.insert(id, plan);
            } else {
                // A nested dependency must not overwrite another method's
                // explicit top-level codec, regardless of service order.
                codecs.messages.entry(id).or_insert(plan);
            }
        }
    }
    let messages = c
        .messages
        .into_iter()
        .filter(|message| codecs.messages.contains_key(&message.id))
        .collect::<Vec<_>>();
    let symbols = Symbols::private(module);
    let mut out = String::new();
    for message in &messages {
        let name = symbols.message(message);
        let ty = type_name(&message.name);
        writeln!(out, "static inline void {name}_clear({ty}_t *);\nstatic inline size_t {name}_encoded_size(const {ty}_t *);\nstatic inline wl_codec_status_t {name}_encode(const {ty}_t *, uint8_t *, size_t, size_t *);\nstatic inline wl_codec_status_t {name}_decode(const uint8_t *, size_t, {ty}_t *);").unwrap();
        if c.maxima[&message.id].is_some() {
            writeln!(out, "static inline void {name}_value_clear({ty}_value_t *);\nstatic inline size_t {name}_value_encoded_size(const {ty}_value_t *);\nstatic inline wl_codec_status_t {name}_value_encode(const {ty}_value_t *, uint8_t *, size_t, size_t *);\nstatic inline wl_codec_status_t {name}_value_decode(const uint8_t *, size_t, {ty}_value_t *);\nstatic inline wl_codec_status_t {name}_value_from_view(const {ty}_t *, {ty}_value_t *);\nstatic inline wl_codec_status_t {name}_value_to_view(const {ty}_value_t *, {ty}_t *);").unwrap();
        }
    }
    let empty = Default::default();
    // Replay hashes the public Schema-v1 business representation. Private wire
    // codecs never supply canonical fingerprints, including in replay-only profiles.
    out.push_str(&emit_source(&messages, &codecs, Some(&empty), &symbols));
    crate::value_codegen::source(&mut out, &messages, &c.maxima, &codecs, &symbols);
    if profile.has_rpc_server() {
        let requests = profile
            .rpc_services
            .iter()
            .filter(|service| {
                service.is_managed()
                    && service.guarantee == crate::profile_semantic::RpcGuarantee::Replay
            })
            .map(|service| service.request_id)
            .collect::<std::collections::BTreeSet<_>>();
        for message in messages
            .iter()
            .filter(|message| requests.contains(&message.id))
        {
            let canonical = plan::MessagePlan::with_profile(message, CodecProfile::SchemaV1);
            let name = format!("{}_wlc_detail_fingerprint", symbols.message(message));
            if fixed::supports(&canonical) {
                out.push_str(&fixed::canonical_fingerprint(&canonical, &name, &symbols));
            } else {
                let ty = type_name(&message.name);
                writeln!(out, "wl_codec_status_t {ty}_wlc_detail_fingerprint(const {ty}_t *, uint64_t *, size_t *);\nstatic inline wl_codec_status_t {name}(const {ty}_t *value, uint64_t *hash, size_t *length) {{ return {ty}_wlc_detail_fingerprint(value, hash, length); }}\n").unwrap();
            }
        }
    }
    out
}
