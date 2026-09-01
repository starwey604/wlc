//! Deterministic C declaration generation for schema v1.

use std::collections::{BTreeSet, HashMap};

use heck::{ToShoutySnakeCase, ToSnakeCase};
use miette::Diagnostic;
use thiserror::Error;

use crate::semantic::{
    FieldDefault, FieldSymbol, MessageSymbol, ResolvedType, SemanticModel, Symbol,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedC {
    pub header: String,
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
    let module = c_identifier(module_name);
    if module.is_empty() {
        return Err(CodegenError(
            "module name has no C identifier characters".to_owned(),
        ));
    }
    validate_names(model)?;
    let guard = format!("WIRELINK_GENERATED_{}_H", upper_snake(&module));
    let mut header = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <wirelink/codec.h>\n"
    );
    let (uses_float32, uses_float64) = ieee_float_usage(model);
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
    let messages = ordered_messages(model)?;
    for message in &messages {
        let name = type_name(&message.name);
        header.push_str(&format!("typedef struct {name} {name}_t;\n"));
    }
    if !messages.is_empty() {
        header.push('\n');
    }
    let static_max_encoded_sizes = static_max_encoded_sizes(&messages);
    for symbol in &model.declarations {
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
        emit_message_definition(&mut header, message);
        header.push('\n');
    }
    for message in &messages {
        let name = type_name(&message.name);
        let macro_name = upper_snake(&message.name);
        header.push_str(&format!(
            "#define {macro_name}_MESSAGE_ID {}U\n",
            message.id,
        ));
        match static_max_encoded_sizes.get(&message.id).copied().flatten() {
            Some(maximum) => header.push_str(&format!(
                "#define {macro_name}_HAS_MAX_ENCODED_SIZE 1\n#define {macro_name}_MAX_ENCODED_SIZE UINT64_C({maximum})\n"
            )),
            None => header.push_str(&format!(
                "#define {macro_name}_HAS_MAX_ENCODED_SIZE 0\n"
            )),
        }
        header.push_str(&format!("void {name}_clear({name}_t *value);\n"));
        header.push_str(&format!(
            "size_t {name}_encoded_size(const {name}_t *value);\n"
        ));
        header.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t out_capacity, size_t *out_length);\n"));
        header.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t input_length, {name}_t *out);\n\n"));
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    let source = emit_source(&module, &messages);
    let bindings_header = emit_bindings_header(&module, &guard, &messages);
    let bindings_source = emit_bindings_source(&module, &messages);
    Ok(GeneratedC {
        header,
        source,
        bindings_header,
        bindings_source,
    })
}

fn static_max_encoded_sizes(messages: &[&MessageSymbol]) -> HashMap<u16, Option<u64>> {
    let messages_by_id = messages
        .iter()
        .map(|message| (message.id, *message))
        .collect::<HashMap<_, _>>();
    let mut memo = HashMap::new();
    let mut visiting = BTreeSet::new();
    for message in messages {
        static_max_encoded_size(message, &messages_by_id, &mut memo, &mut visiting);
    }
    memo
}

fn static_max_encoded_size(
    message: &MessageSymbol,
    messages: &HashMap<u16, &MessageSymbol>,
    memo: &mut HashMap<u16, Option<u64>>,
    visiting: &mut BTreeSet<u16>,
) -> Option<u64> {
    if let Some(maximum) = memo.get(&message.id) {
        return *maximum;
    }
    if !visiting.insert(message.id) {
        memo.insert(message.id, None);
        return None;
    }
    let maximum = (|| {
        let mut total = 0_u64;
        for field in &message.fields {
            let field_maximum = static_max_field_size(field, messages, memo, visiting)?;
            total = total.checked_add(field_maximum)?;
        }
        Some(total)
    })();
    visiting.remove(&message.id);
    memo.insert(message.id, maximum);
    maximum
}

fn static_max_field_size(
    field: &FieldSymbol,
    messages: &HashMap<u16, &MessageSymbol>,
    memo: &mut HashMap<u16, Option<u64>>,
    visiting: &mut BTreeSet<u16>,
) -> Option<u64> {
    if matches!(field.cardinality, crate::ast::Cardinality::Repeated) {
        return None;
    }
    let wire = match field.cardinality {
        crate::ast::Cardinality::Packed(_) => 2,
        _ => wire_type(&field.ty),
    };
    let tag = varint_size((u64::from(field.number) << 3) | wire);
    let body = match field.cardinality {
        crate::ast::Cardinality::Packed(count) => {
            let element_size = match field.ty {
                ResolvedType::Fixed32 | ResolvedType::Float32 => 4_u64,
                ResolvedType::Fixed64 | ResolvedType::Float64 => 8_u64,
                _ => return None,
            };
            let payload = u64::from(count).checked_mul(element_size)?;
            varint_size(payload).checked_add(payload)?
        }
        crate::ast::Cardinality::Optional => match &field.ty {
            ResolvedType::Bool => 1,
            ResolvedType::Uint32 | ResolvedType::Int32 | ResolvedType::Enum { .. } => 5,
            ResolvedType::Uint64 | ResolvedType::Int64 => 10,
            ResolvedType::Fixed32 | ResolvedType::Float32 => 4,
            ResolvedType::Fixed64 | ResolvedType::Float64 => 8,
            ResolvedType::Message { id, .. } => {
                let child = messages.get(id)?;
                let child_maximum = static_max_encoded_size(child, messages, memo, visiting)?;
                varint_size(child_maximum).checked_add(child_maximum)?
            }
            ResolvedType::Bytes | ResolvedType::String => return None,
        },
        crate::ast::Cardinality::Repeated => return None,
    };
    tag.checked_add(body)
}

fn wire_type(ty: &ResolvedType) -> u64 {
    match ty {
        ResolvedType::Fixed64 | ResolvedType::Float64 => 1,
        ResolvedType::Bytes | ResolvedType::String | ResolvedType::Message { .. } => 2,
        ResolvedType::Fixed32 | ResolvedType::Float32 => 5,
        _ => 0,
    }
}

fn varint_size(mut value: u64) -> u64 {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}

fn emit_message_definition(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    output.push_str(&format!("struct {name} {{\n"));
    if message.fields.is_empty() {
        output.push_str("  uint8_t _empty;\n");
    }
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let ty = c_type(&field.ty);
        match field.cardinality {
            crate::ast::Cardinality::Optional => {
                output.push_str(&format!("  bool has_{field_name};\n  {ty} {field_name};\n"));
            }
            crate::ast::Cardinality::Repeated => {
                output.push_str(&format!("  {ty} *{field_name};\n  size_t {field_name}_count;\n  size_t {field_name}_capacity;\n"));
            }
            crate::ast::Cardinality::Packed(count) => {
                output.push_str(&format!(
                    "  bool has_{field_name};\n  {ty} {field_name}[{count}];\n"
                ));
            }
        }
    }
    output.push_str("};\n");
}

fn emit_bindings_header(module: &str, codec_guard: &str, messages: &[&MessageSymbol]) -> String {
    let guard = format!("{codec_guard}_BINDINGS");
    let prefix = upper_snake(module);
    let mut output = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include \"{module}.h\"\n#include <wirelink/wirelink.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    output.push_str(&format!(
        "typedef int32_t {module}_dispatch_domain_t;\nenum {{\n  {prefix}_DISPATCH_OK = 0,\n  {prefix}_DISPATCH_NON_RX,\n  {prefix}_DISPATCH_UNKNOWN_MESSAGE,\n  {prefix}_DISPATCH_MISSING_ROUTE,\n  {prefix}_DISPATCH_MISSING_SCRATCH,\n  {prefix}_DISPATCH_CODEC_ERROR,\n  {prefix}_DISPATCH_HANDLER_ERROR,\n  {prefix}_DISPATCH_INVALID_ARGUMENT\n}};\n\n"
    ));
    output.push_str(&format!(
        "typedef struct {{\n  {module}_dispatch_domain_t domain;\n  uint16_t message_id;\n  wl_event_type_t event_type;\n  wl_codec_status_t codec_status;\n  int32_t handler_result;\n}} {module}_dispatch_result_t;\n\n"
    ));
    output.push_str(&format!(
        "typedef struct {{\n  uint32_t delivered;\n  uint32_t non_rx;\n  uint32_t unknown_message;\n  uint32_t missing_route;\n  uint32_t missing_scratch;\n  uint32_t codec_failure;\n  uint32_t handler_failure;\n}} {module}_dispatch_counters_t;\n\n"
    ));
    output.push_str(&format!(
        "typedef int32_t {module}_send_domain_t;\nenum {{\n  {prefix}_SEND_OK = 0,\n  {prefix}_SEND_CODEC_ERROR,\n  {prefix}_SEND_CORE_ERROR\n}};\n\n"
    ));
    output.push_str(&format!(
        "typedef struct {{\n  uint8_t *data;\n  size_t capacity;\n}} {module}_encode_scratch_t;\n\ntypedef struct {{\n  {module}_send_domain_t domain;\n  wl_codec_status_t codec_status;\n  int core_result;\n  int abort_result;\n  uint32_t core_called;\n  size_t payload_length;\n  wl_tx_handle_t handle;\n}} {module}_send_result_t;\n\n"
    ));
    for message in messages {
        let name = type_name(&message.name);
        output.push_str(&format!(
            "typedef int32_t (*{module}_{name}_handler_fn)(void *user_data, const {name}_t *message, wl_delivery_t delivery);\ntypedef struct {{\n  {name}_t *scratch;\n  {module}_{name}_handler_fn handler;\n  void *user_data;\n}} {module}_{name}_route_t;\n\n"
        ));
    }
    output.push_str("typedef struct {\n");
    for message in messages {
        let name = type_name(&message.name);
        output.push_str(&format!("  {module}_{name}_route_t {name};\n"));
    }
    output.push_str(&format!(
        "  {module}_dispatch_counters_t counters;\n}} {module}_router_t;\n\n"
    ));
    output.push_str(&format!(
        "{module}_dispatch_result_t {module}_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_router_t *router);\n\n"
    ));
    for message in messages {
        let name = type_name(&message.name);
        output.push_str(&format!(
            "{module}_send_result_t {module}_{name}_send_unreliable(wl_ctx_t *ctx, const {name}_t *message, {module}_encode_scratch_t scratch);\n{module}_send_result_t {module}_{name}_send_reliable(wl_ctx_t *ctx, const {name}_t *message, {module}_encode_scratch_t scratch);\n{module}_send_result_t {module}_{name}_send_direct(wl_ctx_t *ctx, const {name}_t *message, wl_delivery_t delivery);\n\n"
        ));
    }
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

fn emit_bindings_source(module: &str, messages: &[&MessageSymbol]) -> String {
    let prefix = upper_snake(module);
    let mut output = format!(
        "#include \"{module}_bindings.h\"\n\n#include <limits.h>\n\nstatic void {module}_count(uint32_t *counter) {{\n  if (*counter != UINT32_MAX) ++*counter;\n}}\n\n"
    );
    output.push_str(&format!(
        "{module}_dispatch_result_t {module}_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_router_t *router) {{\n  {module}_dispatch_result_t result = {{ {prefix}_DISPATCH_INVALID_ARGUMENT, 0U, WL_EVT_NONE, WL_CODEC_OK, 0 }};\n  if (event == NULL) return result;\n  result.message_id = event->message_id;\n  result.event_type = event->type;\n  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_DISPATCH_NON_RX;\n    if (router != NULL) {module}_count(&router->counters.non_rx);\n    return result;\n  }}\n  if (ctx == NULL) return result;\n  switch (event->message_id) {{\n"
    ));
    for message in messages {
        let name = type_name(&message.name);
        let macro_name = upper_snake(&message.name);
        output.push_str(&format!(
            "    case {macro_name}_MESSAGE_ID:\n      if (router == NULL || router->{name}.handler == NULL) {{\n        result.domain = {prefix}_DISPATCH_MISSING_ROUTE;\n        if (router != NULL) {module}_count(&router->counters.missing_route);\n        break;\n      }}\n      if (router->{name}.scratch == NULL) {{\n        result.domain = {prefix}_DISPATCH_MISSING_SCRATCH;\n        {module}_count(&router->counters.missing_scratch);\n        break;\n      }}\n      result.codec_status = {name}_decode(event->payload, event->payload_len, router->{name}.scratch);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_DISPATCH_CODEC_ERROR;\n        {module}_count(&router->counters.codec_failure);\n        break;\n      }}\n      result.handler_result = router->{name}.handler(router->{name}.user_data, router->{name}.scratch, event->type == WL_EVT_RELIABLE_RX ? WL_DELIVERY_RELIABLE : WL_DELIVERY_UNRELIABLE);\n      if (result.handler_result != 0) {{\n        result.domain = {prefix}_DISPATCH_HANDLER_ERROR;\n        {module}_count(&router->counters.handler_failure);\n        break;\n      }}\n      result.domain = {prefix}_DISPATCH_OK;\n      {module}_count(&router->counters.delivered);\n      break;\n"
        ));
    }
    output.push_str(&format!(
        "    default:\n      result.domain = {prefix}_DISPATCH_UNKNOWN_MESSAGE;\n      if (router != NULL) {module}_count(&router->counters.unknown_message);\n      break;\n  }}\n  wl_event_release(ctx, event);\n  return result;\n}}\n\n"
    ));
    for message in messages {
        let name = type_name(&message.name);
        let macro_name = upper_snake(&message.name);
        for (delivery, call) in [
            (
                "unreliable",
                format!(
                    "wl_send_unreliable(ctx, {macro_name}_MESSAGE_ID, scratch.data, result.payload_length)"
                ),
            ),
            (
                "reliable",
                format!(
                    "wl_send_reliable(ctx, {macro_name}_MESSAGE_ID, scratch.data, result.payload_length, &result.handle)"
                ),
            ),
        ] {
            output.push_str(&format!(
                "{module}_send_result_t {module}_{name}_send_{delivery}(wl_ctx_t *ctx, const {name}_t *message, {module}_encode_scratch_t scratch) {{\n  {module}_send_result_t result = {{ {prefix}_SEND_CODEC_ERROR, WL_CODEC_OK, WL_OK, WL_OK, 0U, 0U, 0U }};\n  result.codec_status = {name}_encode(message, scratch.data, scratch.capacity, &result.payload_length);\n  if (result.codec_status != WL_CODEC_OK) return result;\n  result.core_called = 1U;\n  result.core_result = {call};\n  result.domain = result.core_result == WL_OK ? {prefix}_SEND_OK : {prefix}_SEND_CORE_ERROR;\n  return result;\n}}\n\n"
            ));
        }
        output.push_str(&format!(
            "{module}_send_result_t {module}_{name}_send_direct(wl_ctx_t *ctx, const {name}_t *message, wl_delivery_t delivery) {{\n  {module}_send_result_t result = {{ {prefix}_SEND_CORE_ERROR, WL_CODEC_OK, WL_OK, WL_OK, 1U, 0U, 0U }};\n  wl_tx_payload_claim_t claim = {{0}};\n  result.core_result = wl_tx_payload_claim(ctx, {macro_name}_MESSAGE_ID, delivery, &claim);\n  if (result.core_result != WL_OK) return result;\n  result.codec_status = {name}_encode(message, claim.span.data, claim.span.length, &result.payload_length);\n  if (result.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_SEND_CODEC_ERROR;\n    result.abort_result = wl_tx_payload_abort(ctx, &claim);\n    return result;\n  }}\n  result.core_result = wl_tx_payload_commit(ctx, &claim, result.payload_length, delivery == WL_DELIVERY_RELIABLE ? &result.handle : NULL);\n  if (result.core_result != WL_OK) {{\n    result.abort_result = wl_tx_payload_abort(ctx, &claim);\n    return result;\n  }}\n  result.domain = {prefix}_SEND_OK;\n  return result;\n}}\n\n"
        ));
    }
    while output.ends_with("\n\n") {
        output.pop();
    }
    output
}

fn emit_source(module: &str, messages: &[&MessageSymbol]) -> String {
    let mut source = COMMON_C.replace("@MODULE@", module);
    for message in messages {
        source.push_str(&format!(
            "static const wlc_desc_t {}_desc;\n",
            type_name(&message.name)
        ));
    }
    source.push('\n');
    for message in messages {
        emit_descriptor(&mut source, message);
    }
    for message in messages {
        let name = type_name(&message.name);
        source.push_str(&format!("void {name}_clear({name}_t *value) {{ if (value != NULL) wlc_clear(&{name}_desc, value); }}\n"));
        source.push_str(&format!("size_t {name}_encoded_size(const {name}_t *value) {{ size_t size; return wlc_measure(&{name}_desc, value, &size) == WL_CODEC_OK ? size : SIZE_MAX; }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_encode(const {name}_t *value, uint8_t *out, size_t cap, size_t *length) {{ return wlc_encode(&{name}_desc, value, out, cap, length); }}\n"));
        source.push_str(&format!("wl_codec_status_t {name}_decode(const uint8_t *input, size_t length, {name}_t *out) {{ return wlc_decode(&{name}_desc, input, length, out); }}\n\n"));
    }
    source
}

fn emit_descriptor(output: &mut String, message: &MessageSymbol) {
    let name = type_name(&message.name);
    if message.fields.is_empty() {
        output.push_str(&format!(
            "static const wlc_desc_t {name}_desc = {{ NULL, 0U }};\n\n"
        ));
        return;
    }
    output.push_str(&format!("static const wlc_field_t {name}_fields[] = {{\n"));
    for field in &message.fields {
        let field_name = c_identifier(&field.name);
        let (kind, signed_default, unsigned_default, string_default, nested) =
            field_descriptor_data(field);
        let (cardinality, has, count, capacity, packed_count) = match field.cardinality {
            crate::ast::Cardinality::Optional => (
                "WLC_OPTIONAL",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Repeated => (
                "WLC_REPEATED",
                "0".to_owned(),
                format!("offsetof({name}_t, {field_name}_count)"),
                format!("offsetof({name}_t, {field_name}_capacity)"),
                "0".to_owned(),
            ),
            crate::ast::Cardinality::Packed(element_count) => (
                "WLC_PACKED",
                format!("offsetof({name}_t, has_{field_name})"),
                "0".to_owned(),
                "0".to_owned(),
                element_count.to_string(),
            ),
        };
        output.push_str(&format!("  {{ {}U, {cardinality}, {kind}, offsetof({name}_t, {field_name}), {has}, {count}, {capacity}, sizeof({}), {packed_count}U, {signed_default}, {unsigned_default}ULL, {string_default}, {nested} }},\n", field.number, c_type(&field.ty)));
    }
    output.push_str("};\n");
    output.push_str(&format!("static const wlc_desc_t {name}_desc = {{ {name}_fields, sizeof({name}_fields) / sizeof({name}_fields[0]) }};\n\n"));
}

fn field_descriptor_data(field: &FieldSymbol) -> (&'static str, String, String, String, String) {
    let kind = match field.ty {
        ResolvedType::Bool => "WLC_BOOL",
        ResolvedType::Bytes => "WLC_BYTES",
        ResolvedType::String => "WLC_STRING",
        ResolvedType::Int32 => "WLC_I32",
        ResolvedType::Uint32 => "WLC_U32",
        ResolvedType::Int64 => "WLC_I64",
        ResolvedType::Uint64 => "WLC_U64",
        ResolvedType::Fixed32 => "WLC_F32",
        ResolvedType::Fixed64 => "WLC_F64",
        ResolvedType::Float32 => "WLC_FLOAT32",
        ResolvedType::Float64 => "WLC_FLOAT64",
        ResolvedType::Enum { .. } => "WLC_ENUM",
        ResolvedType::Message { .. } => "WLC_MESSAGE",
    };
    let nested = match &field.ty {
        ResolvedType::Message { name, .. } => format!("&{}_desc", type_name(name)),
        _ => "NULL".to_owned(),
    };
    let (signed, unsigned, string) = match &field.default {
        Some(FieldDefault::Bool(value)) => (
            "0".to_owned(),
            (*value as u8).to_string(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::String(value)) => {
            ("0".to_owned(), value.len().to_string(), c_string(value))
        }
        Some(FieldDefault::Int32(value)) | Some(FieldDefault::Enum(value)) => (
            format!("INT32_C({value})"),
            "0".to_owned(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::Int64(value)) => (
            if *value == i64::MIN {
                "INT64_MIN".to_owned()
            } else {
                format!("INT64_C({value})")
            },
            "0".to_owned(),
            "NULL".to_owned(),
        ),
        Some(FieldDefault::Uint32(value)) | Some(FieldDefault::Fixed32(value)) => {
            ("0".to_owned(), value.to_string(), "NULL".to_owned())
        }
        Some(FieldDefault::Uint64(value)) | Some(FieldDefault::Fixed64(value)) => {
            ("0".to_owned(), value.to_string(), "NULL".to_owned())
        }
        None => ("0".to_owned(), "0".to_owned(), "NULL".to_owned()),
    };
    (kind, signed, unsigned, string, nested)
}

const COMMON_C: &str = r#"#include "@MODULE@.h"

#include <limits.h>
#include <stddef.h>
#include <string.h>

enum {
  WLC_OPTIONAL,
  WLC_REPEATED,
  WLC_PACKED,
  WLC_BOOL,
  WLC_U32,
  WLC_U64,
  WLC_I32,
  WLC_I64,
  WLC_F32,
  WLC_F64,
  WLC_FLOAT32,
  WLC_FLOAT64,
  WLC_BYTES,
  WLC_STRING,
  WLC_ENUM,
  WLC_MESSAGE
};
typedef struct wlc_desc wlc_desc_t;
typedef struct {
  uint16_t number;
  uint8_t card, kind;
  size_t value, has, count, capacity, element, packed_count;
  int64_t signed_default;
  uint64_t unsigned_default;
  const char *string_default;
  const wlc_desc_t *nested;
} wlc_field_t;
struct wlc_desc { const wlc_field_t *fields; size_t count; };

static inline wl_codec_status_t wlc_add(size_t *a, size_t b) {
  if (b > SIZE_MAX - *a) return WL_CODEC_ERR_OVERFLOW;
  *a += b;
  return WL_CODEC_OK;
}
static inline size_t wlc_vsize(uint64_t v) {
  size_t n = 1U;
  while (v >= 128U) { v >>= 7U; ++n; }
  return n;
}
static inline void wlc_putv(uint8_t **p, uint64_t v) {
  while (v >= 128U) { *(*p)++ = (uint8_t)(v | 128U); v >>= 7U; }
  *(*p)++ = (uint8_t)v;
}
static wl_codec_status_t wlc_getv(const uint8_t *in, size_t length, size_t *at,
                                  uint64_t *out) {
  size_t start = *at, n = 0U;
  uint64_t v = 0U;
  while (*at < length && n < 10U) {
    uint8_t b = in[(*at)++];
    if (n == 9U && b > 1U) return WL_CODEC_ERR_OVERFLOW;
    v |= (uint64_t)(b & 127U) << (7U * n++);
    if ((b & 128U) == 0U) {
      if (wlc_vsize(v) != *at - start) return WL_CODEC_ERR_MALFORMED;
      *out = v;
      return WL_CODEC_OK;
    }
  }
  return *at == length ? WL_CODEC_ERR_MALFORMED : WL_CODEC_ERR_OVERFLOW;
}
static bool wlc_utf8(const uint8_t *s, size_t n) {
  size_t i = 0U;
  while (i < n) {
    uint8_t a = s[i++];
    if (a < 0x80U) continue;
    size_t need;
    uint32_t v;
    if (a >= 0xC2U && a <= 0xDFU) { need = 1U; v = a & 0x1FU; }
    else if (a >= 0xE0U && a <= 0xEFU) { need = 2U; v = a & 0x0FU; }
    else if (a >= 0xF0U && a <= 0xF4U) { need = 3U; v = a & 0x07U; }
    else return false;
    if (need > n - i) return false;
    while (need-- != 0U) {
      uint8_t b = s[i++];
      if ((b & 0xC0U) != 0x80U) return false;
      v = (v << 6U) | (b & 0x3FU);
    }
    if ((a == 0xE0U && v < 0x800U) ||
        (a == 0xEDU && v >= 0xD800U) ||
        (a == 0xF0U && v < 0x10000U) ||
        (a == 0xF4U && v > 0x10FFFFU)) return false;
  }
  return true;
}
static inline uint8_t wlc_wire(const wlc_field_t *f) {
  if (f->card == WLC_PACKED) return 2U;
  if (f->kind == WLC_F64 || f->kind == WLC_FLOAT64) return 1U;
  if (f->kind == WLC_F32 || f->kind == WLC_FLOAT32) return 5U;
  if (f->kind == WLC_BYTES || f->kind == WLC_STRING || f->kind == WLC_MESSAGE)
    return 2U;
  return 0U;
}
static inline uint64_t wlc_z32(int32_t v) {
  return ((uint32_t)v << 1U) ^ (uint32_t)-(uint32_t)(v < 0);
}
static inline uint64_t wlc_z64(int64_t v) {
  return ((uint64_t)v << 1U) ^ (uint64_t)-(uint64_t)(v < 0);
}
static inline int32_t wlc_uz32(uint32_t v) {
  return (int32_t)((v >> 1U) ^ (uint32_t)-(v & 1U));
}
static inline int64_t wlc_uz64(uint64_t v) {
  return (int64_t)((v >> 1U) ^ (uint64_t)-(v & 1U));
}
static wl_codec_status_t wlc_measure(const wlc_desc_t *, const void *, size_t *);
static void wlc_clear(const wlc_desc_t *, void *);
static wl_codec_status_t wlc_decode(const wlc_desc_t *, const uint8_t *, size_t,
                                    void *);
static wl_codec_status_t wlc_emit_fields(const wlc_desc_t *, const void *,
                                         uint8_t **);

static wl_codec_status_t wlc_packed_bytes(const wlc_field_t *f, size_t *bytes) {
  if (f->packed_count != 0U && f->element > SIZE_MAX / f->packed_count)
    return WL_CODEC_ERR_OVERFLOW;
  *bytes = f->element * f->packed_count;
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_body(const wlc_field_t *f, const void *p,
                                  size_t *n) {
  *n = 0U;
  switch (f->kind) {
    case WLC_BOOL: {
      bool v = *(const bool *)p;
      if (v != false && v != true) return WL_CODEC_ERR_INVALID_VALUE;
      *n = 1U;
      return WL_CODEC_OK;
    }
    case WLC_U32: *n = wlc_vsize(*(const uint32_t *)p); return WL_CODEC_OK;
    case WLC_U64: *n = wlc_vsize(*(const uint64_t *)p); return WL_CODEC_OK;
    case WLC_I32:
    case WLC_ENUM: *n = wlc_vsize(wlc_z32(*(const int32_t *)p)); return WL_CODEC_OK;
    case WLC_I64: *n = wlc_vsize(wlc_z64(*(const int64_t *)p)); return WL_CODEC_OK;
    case WLC_F32:
    case WLC_FLOAT32: *n = 4U; return WL_CODEC_OK;
    case WLC_F64:
    case WLC_FLOAT64: *n = 8U; return WL_CODEC_OK;
    case WLC_BYTES: {
      const wl_codec_bytes_t *v = p;
      if (v->length != 0U && v->data == NULL) return WL_CODEC_ERR_INVALID_VALUE;
      if (wlc_add(n, wlc_vsize(v->length)) != WL_CODEC_OK)
        return WL_CODEC_ERR_OVERFLOW;
      return wlc_add(n, v->length);
    }
    case WLC_STRING: {
      const wl_codec_string_t *v = p;
      if (v->length != 0U && v->data == NULL) return WL_CODEC_ERR_INVALID_VALUE;
      if (!wlc_utf8((const uint8_t *)v->data, v->length)) return WL_CODEC_ERR_UTF8;
      if (wlc_add(n, wlc_vsize(v->length)) != WL_CODEC_OK)
        return WL_CODEC_ERR_OVERFLOW;
      return wlc_add(n, v->length);
    }
    case WLC_MESSAGE: {
      size_t child;
      wl_codec_status_t s = wlc_measure(f->nested, p, &child);
      if (s != WL_CODEC_OK) return s;
      *n = wlc_vsize(child);
      return wlc_add(n, child);
    }
    default: return WL_CODEC_ERR_INVALID_VALUE;
  }
}
static wl_codec_status_t wlc_measure(const wlc_desc_t *d, const void *value,
                                     size_t *out) {
  size_t n = 0U;
  if (d == NULL || value == NULL || out == NULL) return WL_CODEC_ERR_INVALID_VALUE;
  for (size_t i = 0U; i < d->count; ++i) {
    const wlc_field_t *f = &d->fields[i];
    const uint8_t *base = value;
    if (f->card == WLC_PACKED) {
      size_t bytes;
      wl_codec_status_t s;
      if (!*(const bool *)(base + f->has)) continue;
      if ((s = wlc_packed_bytes(f, &bytes)) != WL_CODEC_OK) return s;
      if ((s = wlc_add(&n, wlc_vsize(((uint64_t)f->number << 3U) | 2U))) != WL_CODEC_OK ||
          (s = wlc_add(&n, wlc_vsize(bytes))) != WL_CODEC_OK ||
          (s = wlc_add(&n, bytes)) != WL_CODEC_OK) return s;
      continue;
    }
    size_t count = 1U;
    if (f->card == WLC_OPTIONAL) {
      if (!*(const bool *)(base + f->has)) continue;
    } else {
      count = *(const size_t *)(base + f->count);
      if ((count != 0U && *(void *const *)(base + f->value) == NULL) ||
          count > *(const size_t *)(base + f->capacity))
        return WL_CODEC_ERR_INVALID_VALUE;
    }
    for (size_t j = 0U; j < count; ++j) {
      size_t body;
      const void *p = f->card == WLC_REPEATED
                          ? *(const uint8_t *const *)(base + f->value) + j * f->element
                          : base + f->value;
      wl_codec_status_t s = wlc_body(f, p, &body);
      if (s != WL_CODEC_OK) return s;
      if ((s = wlc_add(&n, wlc_vsize(((uint64_t)f->number << 3U) | wlc_wire(f)))) != WL_CODEC_OK ||
          (s = wlc_add(&n, body)) != WL_CODEC_OK) return s;
    }
  }
  *out = n;
  return WL_CODEC_OK;
}
static void wlc_clear(const wlc_desc_t *d, void *value) {
  uint8_t *base = value;
  for (size_t i = 0U; i < d->count; ++i) {
    const wlc_field_t *f = &d->fields[i];
    void *p = base + f->value;
    if (f->card == WLC_REPEATED) {
      *(size_t *)(base + f->count) = 0U;
      continue;
    }
    *(bool *)(base + f->has) = false;
    if (f->card == WLC_PACKED) {
      memset(p, 0, f->element * f->packed_count);
      continue;
    }
    if (f->kind == WLC_MESSAGE) { wlc_clear(f->nested, p); continue; }
    if (f->kind == WLC_STRING) {
      *(wl_codec_string_t *)p =
          (wl_codec_string_t){f->string_default, (size_t)f->unsigned_default};
      continue;
    }
    if (f->kind == WLC_BYTES) {
      *(wl_codec_bytes_t *)p = (wl_codec_bytes_t){NULL, 0U};
      continue;
    }
    if (f->kind == WLC_FLOAT32 || f->kind == WLC_FLOAT64) {
      memset(p, 0, f->element);
      continue;
    }
    if (f->kind == WLC_BOOL) *(bool *)p = f->unsigned_default != 0U;
    else if (f->kind == WLC_I32 || f->kind == WLC_ENUM)
      *(int32_t *)p = (int32_t)f->signed_default;
    else if (f->kind == WLC_I64) *(int64_t *)p = f->signed_default;
    else if (f->kind == WLC_U32 || f->kind == WLC_F32)
      *(uint32_t *)p = (uint32_t)f->unsigned_default;
    else if (f->kind == WLC_U64 || f->kind == WLC_F64)
      *(uint64_t *)p = f->unsigned_default;
  }
}
static void wlc_put_fixed(uint8_t **p, uint64_t v, size_t n) {
  while (n-- != 0U) *(*p)++ = (uint8_t)(v >> (8U * n));
}
static wl_codec_status_t wlc_emit_fixed(uint8_t kind, const void *value,
                                        uint8_t **out) {
  uint64_t bits;
  if (kind == WLC_F32) bits = *(const uint32_t *)value;
  else if (kind == WLC_F64) bits = *(const uint64_t *)value;
  else if (kind == WLC_FLOAT32) {
    uint32_t bits32;
    memcpy(&bits32, value, sizeof(bits32));
    bits = bits32;
  } else if (kind == WLC_FLOAT64) {
    memcpy(&bits, value, sizeof(bits));
  } else return WL_CODEC_ERR_INVALID_VALUE;
  wlc_put_fixed(out, bits, (kind == WLC_F32 || kind == WLC_FLOAT32) ? 4U : 8U);
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_emit_value(const wlc_field_t *f, const void *p,
                                        uint8_t **out) {
  switch (f->kind) {
    case WLC_BOOL: wlc_putv(out, *(const bool *)p); return WL_CODEC_OK;
    case WLC_U32: wlc_putv(out, *(const uint32_t *)p); return WL_CODEC_OK;
    case WLC_U64: wlc_putv(out, *(const uint64_t *)p); return WL_CODEC_OK;
    case WLC_I32:
    case WLC_ENUM: wlc_putv(out, wlc_z32(*(const int32_t *)p)); return WL_CODEC_OK;
    case WLC_I64: wlc_putv(out, wlc_z64(*(const int64_t *)p)); return WL_CODEC_OK;
    case WLC_F32:
    case WLC_F64:
    case WLC_FLOAT32:
    case WLC_FLOAT64: return wlc_emit_fixed(f->kind, p, out);
    case WLC_BYTES: {
      const wl_codec_bytes_t *v = p;
      wlc_putv(out, v->length);
      if (v->length != 0U) { memcpy(*out, v->data, v->length); *out += v->length; }
      return WL_CODEC_OK;
    }
    case WLC_STRING: {
      const wl_codec_string_t *v = p;
      wlc_putv(out, v->length);
      if (v->length != 0U) { memcpy(*out, v->data, v->length); *out += v->length; }
      return WL_CODEC_OK;
    }
    case WLC_MESSAGE: {
      size_t child;
      wl_codec_status_t s = wlc_measure(f->nested, p, &child);
      if (s != WL_CODEC_OK) return s;
      wlc_putv(out, child);
      return wlc_emit_fields(f->nested, p, out);
    }
    default: return WL_CODEC_ERR_INVALID_VALUE;
  }
}
static wl_codec_status_t wlc_emit_packed(const wlc_field_t *f, const void *p,
                                         uint8_t **out) {
  size_t bytes;
  wl_codec_status_t s = wlc_packed_bytes(f, &bytes);
  if (s != WL_CODEC_OK) return s;
  wlc_putv(out, bytes);
  for (size_t j = 0U; j < f->packed_count; ++j) {
    if ((s = wlc_emit_fixed(f->kind, (const uint8_t *)p + j * f->element, out))
        != WL_CODEC_OK) return s;
  }
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_emit_fields(const wlc_desc_t *d, const void *value,
                                         uint8_t **out) {
  for (size_t i = 0U; i < d->count; ++i) {
    const wlc_field_t *f = &d->fields[i];
    const uint8_t *base = value;
    if (f->card == WLC_PACKED) {
      wl_codec_status_t s;
      if (!*(const bool *)(base + f->has)) continue;
      wlc_putv(out, ((uint64_t)f->number << 3U) | 2U);
      if ((s = wlc_emit_packed(f, base + f->value, out)) != WL_CODEC_OK) return s;
      continue;
    }
    size_t count = f->card == WLC_OPTIONAL
                       ? (*(const bool *)(base + f->has) ? 1U : 0U)
                       : *(const size_t *)(base + f->count);
    for (size_t j = 0U; j < count; ++j) {
      const void *p = f->card == WLC_REPEATED
                          ? *(const uint8_t *const *)(base + f->value) + j * f->element
                          : base + f->value;
      wl_codec_status_t s;
      wlc_putv(out, ((uint64_t)f->number << 3U) | wlc_wire(f));
      if ((s = wlc_emit_value(f, p, out)) != WL_CODEC_OK) return s;
    }
  }
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_encode(const wlc_desc_t *d, const void *value,
                                    uint8_t *out, size_t cap, size_t *length) {
  size_t n;
  wl_codec_status_t s = wlc_measure(d, value, &n);
  if (s != WL_CODEC_OK || length == NULL || (n != 0U && out == NULL))
    return s == WL_CODEC_OK ? WL_CODEC_ERR_INVALID_VALUE : s;
  if (cap < n) return WL_CODEC_ERR_CAPACITY;
  uint8_t *p = out;
  if ((s = wlc_emit_fields(d, value, &p)) != WL_CODEC_OK) return s;
  *length = n;
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_skip(uint8_t wire, const uint8_t *in, size_t n,
                                  size_t *at) {
  uint64_t length;
  wl_codec_status_t s;
  if (wire == 0U) return wlc_getv(in, n, at, &length);
  if (wire == 1U) {
    if (n - *at < 8U) return WL_CODEC_ERR_MALFORMED;
    *at += 8U;
    return WL_CODEC_OK;
  }
  if (wire == 5U) {
    if (n - *at < 4U) return WL_CODEC_ERR_MALFORMED;
    *at += 4U;
    return WL_CODEC_OK;
  }
  if (wire != 2U) return WL_CODEC_ERR_MALFORMED;
  if ((s = wlc_getv(in, n, at, &length)) != WL_CODEC_OK) return s;
  if (length > n - *at) return WL_CODEC_ERR_MALFORMED;
  *at += (size_t)length;
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_read_fixed(uint8_t kind, const uint8_t *in,
                                        size_t n, size_t *at, void *out) {
  size_t bytes = (kind == WLC_F32 || kind == WLC_FLOAT32) ? 4U : 8U;
  if (n - *at < bytes) return WL_CODEC_ERR_MALFORMED;
  uint64_t bits = 0U;
  for (size_t i = 0U; i < bytes; ++i) bits = (bits << 8U) | in[(*at)++];
  if (kind == WLC_F32) *(uint32_t *)out = (uint32_t)bits;
  else if (kind == WLC_F64) *(uint64_t *)out = bits;
  else if (kind == WLC_FLOAT32) {
    uint32_t bits32 = (uint32_t)bits;
    memcpy(out, &bits32, sizeof(bits32));
  } else if (kind == WLC_FLOAT64) memcpy(out, &bits, sizeof(bits));
  else return WL_CODEC_ERR_INVALID_VALUE;
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_read_value(const wlc_field_t *f, const uint8_t *in,
                                        size_t n, size_t *at, void *out) {
  uint64_t v;
  wl_codec_status_t s;
  if (f->kind == WLC_F32 || f->kind == WLC_F64 ||
      f->kind == WLC_FLOAT32 || f->kind == WLC_FLOAT64)
    return wlc_read_fixed(f->kind, in, n, at, out);
  if (f->kind == WLC_BYTES || f->kind == WLC_STRING || f->kind == WLC_MESSAGE) {
    if ((s = wlc_getv(in, n, at, &v)) != WL_CODEC_OK) return s;
    if (v > n - *at) return WL_CODEC_ERR_MALFORMED;
    size_t bytes = (size_t)v;
    if (f->kind == WLC_BYTES)
      *(wl_codec_bytes_t *)out = (wl_codec_bytes_t){in + *at, bytes};
    else if (f->kind == WLC_STRING) {
      if (!wlc_utf8(in + *at, bytes)) return WL_CODEC_ERR_UTF8;
      *(wl_codec_string_t *)out =
          (wl_codec_string_t){(const char *)(in + *at), bytes};
    } else {
      s = wlc_decode(f->nested, in + *at, bytes, out);
      if (s != WL_CODEC_OK) return s;
    }
    *at += bytes;
    return WL_CODEC_OK;
  }
  if ((s = wlc_getv(in, n, at, &v)) != WL_CODEC_OK) return s;
  if (f->kind == WLC_BOOL) {
    if (v > 1U) return WL_CODEC_ERR_INVALID_VALUE;
    *(bool *)out = v != 0U;
  } else if (f->kind == WLC_U32) {
    if (v > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW;
    *(uint32_t *)out = (uint32_t)v;
  } else if (f->kind == WLC_U64) *(uint64_t *)out = v;
  else if (f->kind == WLC_I32 || f->kind == WLC_ENUM) {
    if (v > UINT32_MAX) return WL_CODEC_ERR_OVERFLOW;
    *(int32_t *)out = wlc_uz32((uint32_t)v);
  } else if (f->kind == WLC_I64) *(int64_t *)out = wlc_uz64(v);
  else return WL_CODEC_ERR_INVALID_VALUE;
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_read_packed(const wlc_field_t *f,
                                         const uint8_t *in, size_t n,
                                         size_t *at, void *out) {
  uint64_t encoded_bytes;
  size_t expected_bytes;
  wl_codec_status_t s;
  if ((s = wlc_getv(in, n, at, &encoded_bytes)) != WL_CODEC_OK) return s;
  if ((s = wlc_packed_bytes(f, &expected_bytes)) != WL_CODEC_OK) return s;
  if (encoded_bytes != expected_bytes || expected_bytes > n - *at)
    return WL_CODEC_ERR_MALFORMED;
  for (size_t j = 0U; j < f->packed_count; ++j) {
    if ((s = wlc_read_fixed(f->kind, in, n, at,
                            (uint8_t *)out + j * f->element)) != WL_CODEC_OK)
      return s;
  }
  return WL_CODEC_OK;
}
static wl_codec_status_t wlc_decode(const wlc_desc_t *d, const uint8_t *in,
                                    size_t n, void *out) {
  if (d == NULL || out == NULL || (n != 0U && in == NULL))
    return WL_CODEC_ERR_INVALID_VALUE;
  wlc_clear(d, out);
  for (size_t at = 0U; at < n;) {
    uint64_t key;
    wl_codec_status_t s = wlc_getv(in, n, &at, &key);
    if (s != WL_CODEC_OK) return s;
    uint64_t number = key >> 3U;
    uint8_t wire = (uint8_t)(key & 7U);
    if (number == 0U || number > 65535U ||
        (wire != 0U && wire != 1U && wire != 2U && wire != 5U))
      return WL_CODEC_ERR_MALFORMED;
    const wlc_field_t *f = NULL;
    for (size_t i = 0U; i < d->count; ++i) {
      if (d->fields[i].number == number) { f = &d->fields[i]; break; }
    }
    if (f == NULL) {
      if ((s = wlc_skip(wire, in, n, &at)) != WL_CODEC_OK) return s;
      continue;
    }
    if (wire != wlc_wire(f)) return WL_CODEC_ERR_WIRE_TYPE;
    uint8_t *base = out;
    if (f->card == WLC_PACKED) {
      if (*(bool *)(base + f->has)) return WL_CODEC_ERR_DUPLICATE_FIELD;
      if ((s = wlc_read_packed(f, in, n, &at, base + f->value)) != WL_CODEC_OK)
        return s;
      *(bool *)(base + f->has) = true;
      continue;
    }
    void *p;
    if (f->card == WLC_OPTIONAL) {
      if (*(bool *)(base + f->has)) return WL_CODEC_ERR_DUPLICATE_FIELD;
      p = base + f->value;
    } else {
      size_t count = *(size_t *)(base + f->count);
      if (count >= *(size_t *)(base + f->capacity)) return WL_CODEC_ERR_CAPACITY;
      void *storage = *(void **)(base + f->value);
      if (storage == NULL) return WL_CODEC_ERR_INVALID_VALUE;
      p = (uint8_t *)storage + count * f->element;
    }
    if ((s = wlc_read_value(f, in, n, &at, p)) != WL_CODEC_OK) return s;
    if (f->card == WLC_OPTIONAL) *(bool *)(base + f->has) = true;
    else ++*(size_t *)(base + f->count);
  }
  return WL_CODEC_OK;
}
"#;

fn c_string(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\"\\x{byte:02X}\""))
        .collect()
}

fn c_type(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Bool => "bool".to_owned(),
        ResolvedType::Bytes => "wl_codec_bytes_t".to_owned(),
        ResolvedType::String => "wl_codec_string_t".to_owned(),
        ResolvedType::Int32 => "int32_t".to_owned(),
        ResolvedType::Uint32 | ResolvedType::Fixed32 => "uint32_t".to_owned(),
        ResolvedType::Float32 => "float".to_owned(),
        ResolvedType::Int64 => "int64_t".to_owned(),
        ResolvedType::Uint64 | ResolvedType::Fixed64 => "uint64_t".to_owned(),
        ResolvedType::Float64 => "double".to_owned(),
        ResolvedType::Message { name, .. } | ResolvedType::Enum { name, .. } => {
            format!("{}_t", type_name(name))
        }
    }
}

fn ieee_float_usage(model: &SemanticModel) -> (bool, bool) {
    let mut float32 = false;
    let mut float64 = false;
    for field in model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some(message.fields.as_slice()),
            Symbol::Enum(_) => None,
        })
        .flatten()
    {
        float32 |= matches!(field.ty, ResolvedType::Float32);
        float64 |= matches!(field.ty, ResolvedType::Float64);
    }
    (float32, float64)
}

fn ordered_messages(model: &SemanticModel) -> Result<Vec<&MessageSymbol>, CodegenError> {
    let messages: HashMap<&str, &MessageSymbol> = model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.name.as_str(), message)),
            _ => None,
        })
        .collect();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::new();
    fn visit<'a>(
        message: &'a MessageSymbol,
        messages: &HashMap<&'a str, &'a MessageSymbol>,
        emitted: &mut BTreeSet<&'a str>,
        ordered: &mut Vec<&'a MessageSymbol>,
    ) {
        if !emitted.insert(message.name.as_str()) {
            return;
        }
        for field in &message.fields {
            if let ResolvedType::Message { name, .. } = &field.ty
                && let Some(child) = messages.get(name.as_str())
            {
                visit(child, messages, emitted, ordered);
            }
        }
        ordered.push(message);
    }
    for symbol in &model.declarations {
        if let Symbol::Message(message) = symbol {
            visit(message, &messages, &mut emitted, &mut ordered);
        }
    }
    Ok(ordered)
}

fn validate_names(model: &SemanticModel) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for symbol in &model.declarations {
        let name = type_name(symbol.name());
        if !names.insert(name.clone()) {
            return Err(CodegenError(format!(
                "declarations collide as C identifier `{name}`"
            )));
        }
    }
    let messages = model
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some(message),
            Symbol::Enum(_) => None,
        })
        .collect::<Vec<_>>();
    let maxima = static_max_encoded_sizes(&messages);
    let mut macros = BTreeSet::new();
    for symbol in &model.declarations {
        match symbol {
            Symbol::Message(message) => {
                let prefix = upper_snake(&message.name);
                let mut generated = vec![
                    format!("{prefix}_MESSAGE_ID"),
                    format!("{prefix}_HAS_MAX_ENCODED_SIZE"),
                ];
                if maxima.get(&message.id).copied().flatten().is_some() {
                    generated.push(format!("{prefix}_MAX_ENCODED_SIZE"));
                }
                for name in generated {
                    if !macros.insert(name.clone()) {
                        return Err(CodegenError(format!(
                            "generated macros collide as C identifier `{name}`"
                        )));
                    }
                }
            }
            Symbol::Enum(enumeration) => {
                for value in &enumeration.values {
                    let name = upper_snake(&value.name);
                    if !macros.insert(name.clone()) {
                        return Err(CodegenError(format!(
                            "generated macros collide as C identifier `{name}`"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn type_name(name: &str) -> String {
    c_identifier(name)
}
pub(crate) fn c_identifier(name: &str) -> String {
    let mut output: String = name
        .to_snake_case()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    if output.is_empty() {
        return output;
    }
    if output.as_bytes()[0].is_ascii_digit() {
        output.insert(0, '_');
    }
    if is_c_keyword(&output) {
        output.push('_');
    }
    output
}
pub(crate) fn upper_snake(name: &str) -> String {
    c_identifier(name).to_shouty_snake_case()
}

fn is_c_keyword(name: &str) -> bool {
    matches!(
        name,
        /* C11 keywords, stdbool macros, and C++20 keywords. Generated public
         * headers are required to compile as both strict C11 and C++20. */
        "alignas"
            | "alignof"
            | "and"
            | "and_eq"
            | "asm"
            | "atomic_cancel"
            | "atomic_commit"
            | "atomic_noexcept"
            | "auto"
            | "bitand"
            | "bitor"
            | "bool"
            | "break"
            | "case"
            | "catch"
            | "char"
            | "char8_t"
            | "char16_t"
            | "char32_t"
            | "class"
            | "compl"
            | "concept"
            | "const"
            | "consteval"
            | "constexpr"
            | "constinit"
            | "const_cast"
            | "continue"
            | "co_await"
            | "co_return"
            | "co_yield"
            | "decltype"
            | "default"
            | "delete"
            | "do"
            | "double"
            | "dynamic_cast"
            | "else"
            | "enum"
            | "explicit"
            | "extern"
            | "export"
            | "false"
            | "float"
            | "for"
            | "friend"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "mutable"
            | "namespace"
            | "new"
            | "noexcept"
            | "not"
            | "not_eq"
            | "nullptr"
            | "operator"
            | "or"
            | "or_eq"
            | "private"
            | "protected"
            | "public"
            | "register"
            | "reinterpret_cast"
            | "requires"
            | "restrict"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "static_assert"
            | "static_cast"
            | "struct"
            | "switch"
            | "template"
            | "this"
            | "thread_local"
            | "throw"
            | "true"
            | "try"
            | "typeid"
            | "typename"
            | "typedef"
            | "union"
            | "unsigned"
            | "using"
            | "virtual"
            | "void"
            | "volatile"
            | "wchar_t"
            | "while"
            | "xor"
            | "xor_eq"
            | "_alignas"
            | "_alignof"
            | "_atomic"
            | "_bool"
            | "_complex"
            | "_generic"
            | "_imaginary"
            | "_noreturn"
            | "_static_assert"
            | "_thread_local"
    )
}
