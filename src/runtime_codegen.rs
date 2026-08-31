//! Deterministic C runtime generation for an optional binding profile.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Write,
};

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    ast::Cardinality,
    codegen::{c_identifier, generate_c, type_name, upper_snake},
    identity::{IDENTITY_ALGORITHM, binding_profile_identity, schema_identity},
    profile::BINDING_PROFILE_VERSION,
    profile_semantic::{
        BindingProfileModel, DeliveryPolicy, RetainedRouteKind, RpcService, RpcStatusDomain,
    },
    semantic::{MessageSymbol, ResolvedType, SemanticModel, Symbol},
};

const RPC_FINGERPRINT_ALGORITHM: &str = "fnv1a64-canonical-request-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedRuntimeC {
    pub header: String,
    pub source: String,
}

#[derive(Clone, Debug, Diagnostic, Error, Eq, PartialEq)]
#[error("C runtime generation failed: {0}")]
#[diagnostic(code(wlc::runtime_codegen))]
pub struct RuntimeCodegenError(pub String);

/// Emit an optional application runtime translation unit for a resolved
/// binding profile. The ordinary codec and binding artifacts remain separate
/// and byte-for-byte independent of this function.
pub fn generate_runtime_c(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    module_name: &str,
) -> Result<GeneratedRuntimeC, RuntimeCodegenError> {
    /* Reuse the ordinary generator's complete C-name collision validation. */
    generate_c(schema, module_name).map_err(|error| RuntimeCodegenError(error.0))?;
    let module = c_identifier(module_name);
    if module.is_empty() {
        return Err(RuntimeCodegenError(
            "module name has no C identifier characters".to_owned(),
        ));
    }
    validate_profile_model(schema, profile)?;
    validate_runtime_names(schema, profile, &module)?;

    Ok(GeneratedRuntimeC {
        header: emit_header(schema, profile, &module),
        source: emit_source(profile, &module),
    })
}

fn validate_runtime_names(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    module: &str,
) -> Result<(), RuntimeCodegenError> {
    let prefix = upper_snake(module);
    let mut member_names = BTreeSet::from([
        "_reserved".to_owned(),
        "rpc_client".to_owned(),
        "rpc_server".to_owned(),
    ]);
    for route in &profile.retained_routes {
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        member_names.insert(format!("{}_{kind}", type_name(&route.message_name)));
    }
    let mut runtime_names = BTreeSet::from([
        format!("{module}_runtime_domain_t"),
        format!("{module}_runtime_result_t"),
        format!("{module}_runtime_t"),
        format!("{module}_runtime_dispatch_event"),
        format!("{module}_runtime_result"),
        format!("WIRELINK_GENERATED_{prefix}_RUNTIME_H"),
        format!("{prefix}_SCHEMA_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_VERSION"),
        format!("{prefix}_IDENTITY_ALGORITHM"),
    ]);
    if !profile.rpc_services.is_empty() {
        runtime_names.insert(format!("{module}_rpc_request_fingerprint"));
        runtime_names.insert(format!("{prefix}_RPC_REQUEST_FINGERPRINT_ALGORITHM"));
    }
    for suffix in [
        "OK",
        "NON_RX",
        "UNKNOWN_MESSAGE",
        "MISSING_ROUTE",
        "MISSING_SCRATCH",
        "DELIVERY_MISMATCH",
        "CODEC_ERROR",
        "STORAGE_ERROR",
        "RPC_ERROR",
        "CORE_ERROR",
        "APPLICATION_ERROR",
        "INVALID_ARGUMENT",
    ] {
        runtime_names.insert(format!("{prefix}_RUNTIME_{suffix}"));
    }
    for service in &profile.rpc_services {
        let name = c_identifier(&service.name);
        if name.is_empty() {
            return Err(RuntimeCodegenError(format!(
                "RPC service `{}` has no C identifier characters",
                service.name
            )));
        }
        if !member_names.insert(name.clone()) {
            return Err(RuntimeCodegenError(format!(
                "runtime fields collide as C identifier `{name}`"
            )));
        }
        for symbol in [
            format!("{module}_{name}_request_handler_fn"),
            format!("{module}_{name}_rpc_t"),
            format!("{module}_{name}_client_start_scratch"),
            format!("{module}_{name}_client_start_direct"),
            format!("{module}_{name}_client_finish_start"),
            format!("{module}_{name}_server_complete"),
            format!("{module}_{name}_server_reject"),
            format!("{module}_{name}_server_retry_cached"),
            format!("{module}_{name}_server_finish"),
        ] {
            if !runtime_names.insert(symbol.clone()) {
                return Err(RuntimeCodegenError(format!(
                    "generated runtime symbols collide as C identifier `{symbol}`"
                )));
            }
        }
    }

    let mut schema_names = BTreeSet::new();
    for symbol in &schema.declarations {
        let name = type_name(symbol.name());
        schema_names.insert(format!("{name}_t"));
        match symbol {
            Symbol::Message(message) => {
                schema_names.insert(format!("{name}_clear"));
                schema_names.insert(format!("{name}_encoded_size"));
                schema_names.insert(format!("{name}_encode"));
                schema_names.insert(format!("{name}_decode"));
                schema_names.insert(format!("{}_MESSAGE_ID", upper_snake(&message.name)));
            }
            Symbol::Enum(enumeration) => {
                for value in &enumeration.values {
                    schema_names.insert(upper_snake(&value.name));
                }
            }
        }
    }
    if let Some(collision) = runtime_names.intersection(&schema_names).next() {
        return Err(RuntimeCodegenError(format!(
            "schema and generated runtime collide as C identifier `{collision}`"
        )));
    }
    Ok(())
}

fn validate_profile_model(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
) -> Result<(), RuntimeCodegenError> {
    if profile.version != BINDING_PROFILE_VERSION {
        return Err(RuntimeCodegenError(format!(
            "unsupported binding profile version {}; only version {} is supported",
            profile.version, BINDING_PROFILE_VERSION
        )));
    }
    let messages: HashMap<&str, &MessageSymbol> = schema
        .declarations
        .iter()
        .filter_map(|symbol| match symbol {
            Symbol::Message(message) => Some((message.name.as_str(), message)),
            Symbol::Enum(_) => None,
        })
        .collect();
    let mut retained_ids = HashSet::new();
    for route in &profile.retained_routes {
        let message = exact_message(&messages, &route.message_name, route.message_id)?;
        if !retained_ids.insert(message.id) {
            return Err(RuntimeCodegenError(format!(
                "message `{}` has more than one retained route",
                message.name
            )));
        }
        if let Some(path) = retained_ownership_problem(message, &messages, &mut Vec::new()) {
            return Err(RuntimeCodegenError(format!(
                "message `{}` cannot use a retained {:?} route because `{path}` contains borrowed or caller-owned storage",
                message.name, route.kind
            )));
        }
    }

    let mut service_names = HashSet::new();
    let mut rpc_roles = HashSet::new();
    for service in &profile.rpc_services {
        if !service_names.insert(service.name.as_str()) {
            return Err(RuntimeCodegenError(format!(
                "duplicate RPC service `{}`",
                service.name
            )));
        }
        let request = exact_message(&messages, &service.request_name, service.request_id)?;
        let response = exact_message(&messages, &service.response_name, service.response_id)?;
        if request.id == response.id {
            return Err(RuntimeCodegenError(format!(
                "RPC service `{}` uses one message for both roles",
                service.name
            )));
        }
        for (message, role) in [(request, "request"), (response, "response")] {
            if retained_ids.contains(&message.id) {
                return Err(RuntimeCodegenError(format!(
                    "RPC {role} message `{}` is also a retained route",
                    message.name
                )));
            }
            if !rpc_roles.insert(message.id) {
                return Err(RuntimeCodegenError(format!(
                    "message `{}` is reused by multiple RPC roles",
                    message.name
                )));
            }
        }
        validate_operation_field(
            request,
            &service.request_operation_id.name,
            service.request_operation_id.number,
        )?;
        validate_operation_field(
            response,
            &service.response_operation_id.name,
            service.response_operation_id.number,
        )?;
        validate_status_field(schema, response, service)?;
    }
    Ok(())
}

fn exact_message<'a>(
    messages: &HashMap<&str, &'a MessageSymbol>,
    name: &str,
    id: u16,
) -> Result<&'a MessageSymbol, RuntimeCodegenError> {
    let Some(message) = messages.get(name).copied() else {
        return Err(RuntimeCodegenError(format!(
            "binding profile references missing message `{name}`"
        )));
    };
    if message.id != id {
        return Err(RuntimeCodegenError(format!(
            "binding profile expects message `{name}` ID {id}, but schema uses ID {}",
            message.id
        )));
    }
    Ok(message)
}

fn validate_operation_field(
    message: &MessageSymbol,
    name: &str,
    number: u16,
) -> Result<(), RuntimeCodegenError> {
    let Some(field) = message
        .fields
        .iter()
        .find(|field| field.name == name && field.number == number)
    else {
        return Err(RuntimeCodegenError(format!(
            "binding profile expects operation field `{}.{name}` number {number}",
            message.name
        )));
    };
    if field.cardinality != Cardinality::Optional || field.ty != ResolvedType::Uint32 {
        return Err(RuntimeCodegenError(format!(
            "RPC operation field `{}.{name}` must remain optional uint32",
            message.name
        )));
    }
    Ok(())
}

fn validate_status_field(
    schema: &SemanticModel,
    response: &MessageSymbol,
    service: &RpcService,
) -> Result<(), RuntimeCodegenError> {
    let mapping = &service.response_status;
    let Some(field) = response
        .fields
        .iter()
        .find(|field| field.name == mapping.name && field.number == mapping.number)
    else {
        return Err(RuntimeCodegenError(format!(
            "binding profile expects status field `{}.{}` number {}",
            response.name, mapping.name, mapping.number
        )));
    };
    if field.cardinality != Cardinality::Optional {
        return Err(RuntimeCodegenError(format!(
            "RPC status field `{}.{}` must remain optional",
            response.name, mapping.name
        )));
    }
    match (&field.ty, &service.status_domain) {
        (ResolvedType::Int32, RpcStatusDomain::Int32) => Ok(()),
        (
            ResolvedType::Enum { id, name },
            RpcStatusDomain::Enum {
                id: expected_id,
                name: expected_name,
            },
        ) if id == expected_id && name == expected_name => {
            let has_zero = schema.declarations.iter().any(|symbol| {
                matches!(symbol, Symbol::Enum(value) if value.id == *id && value.name == *name && value.values.iter().any(|variant| variant.number == 0))
            });
            if has_zero {
                Ok(())
            } else {
                Err(RuntimeCodegenError(format!(
                    "RPC status enum `{name}` no longer declares numeric success zero"
                )))
            }
        }
        _ => Err(RuntimeCodegenError(format!(
            "RPC status field `{}.{}` no longer matches its resolved status domain",
            response.name, mapping.name
        ))),
    }
}

fn retained_ownership_problem(
    message: &MessageSymbol,
    messages: &HashMap<&str, &MessageSymbol>,
    stack: &mut Vec<String>,
) -> Option<String> {
    if stack.iter().any(|name| name == &message.name) {
        return Some(message.name.clone());
    }
    stack.push(message.name.clone());
    for field in &message.fields {
        let path = format!("{}.{}", message.name, field.name);
        if field.cardinality == Cardinality::Repeated
            || matches!(field.ty, ResolvedType::Bytes | ResolvedType::String)
        {
            stack.pop();
            return Some(path);
        }
        if let ResolvedType::Message { name, .. } = &field.ty
            && let Some(nested) = messages.get(name.as_str())
            && let Some(nested_path) = retained_ownership_problem(nested, messages, stack)
        {
            stack.pop();
            return Some(format!("{path} -> {nested_path}"));
        }
    }
    stack.pop();
    None
}

fn emit_header(schema: &SemanticModel, profile: &BindingProfileModel, module: &str) -> String {
    let prefix = upper_snake(module);
    let guard = format!("WIRELINK_GENERATED_{prefix}_RUNTIME_H");
    let mut output = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include \"{module}_bindings.h\"\n#include <wirelink/fifo.h>\n#include <wirelink/latest.h>\n#include <wirelink/rpc.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    writeln!(
        output,
        "#define {prefix}_SCHEMA_IDENTITY UINT64_C(0x{:016X})",
        schema_identity(schema)
    )
    .unwrap();
    writeln!(
        output,
        "#define {prefix}_BINDING_PROFILE_IDENTITY UINT64_C(0x{:016X})",
        binding_profile_identity(profile)
    )
    .unwrap();
    writeln!(
        output,
        "#define {prefix}_BINDING_PROFILE_VERSION {}U",
        profile.version
    )
    .unwrap();
    writeln!(
        output,
        "#define {prefix}_IDENTITY_ALGORITHM \"{IDENTITY_ALGORITHM}\"\n"
    )
    .unwrap();
    if !profile.rpc_services.is_empty() {
        writeln!(
            output,
            "#define {prefix}_RPC_REQUEST_FINGERPRINT_ALGORITHM \"{RPC_FINGERPRINT_ALGORITHM}\"\n"
        )
        .unwrap();
    }

    write!(
        output,
        "typedef int32_t {module}_runtime_domain_t;\nenum {{\n  {prefix}_RUNTIME_OK = 0,\n  {prefix}_RUNTIME_NON_RX,\n  {prefix}_RUNTIME_UNKNOWN_MESSAGE,\n  {prefix}_RUNTIME_MISSING_ROUTE,\n  {prefix}_RUNTIME_MISSING_SCRATCH,\n  {prefix}_RUNTIME_DELIVERY_MISMATCH,\n  {prefix}_RUNTIME_CODEC_ERROR,\n  {prefix}_RUNTIME_STORAGE_ERROR,\n  {prefix}_RUNTIME_RPC_ERROR,\n  {prefix}_RUNTIME_CORE_ERROR,\n  {prefix}_RUNTIME_APPLICATION_ERROR,\n  {prefix}_RUNTIME_INVALID_ARGUMENT\n}};\n\n"
    )
    .unwrap();
    write!(
        output,
        "typedef struct {{\n  {module}_runtime_domain_t domain;\n  uint16_t message_id;\n  wl_event_type_t event_type;\n  wl_codec_status_t codec_status;\n  int32_t storage_result;\n  int32_t abort_result;\n  wl_rpc_err_t rpc_result;\n  int32_t core_result;\n  int32_t application_result;\n  wl_rpc_server_disposition_t rpc_disposition;\n  uint32_t operation_id;\n  wl_tx_handle_t handle;\n  size_t payload_length;\n  wl_rpc_server_response_t server_response;\n}} {module}_runtime_result_t;\n\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        emit_rpc_header_types(&mut output, module, service);
    }
    output.push_str("typedef struct {\n  uint8_t _reserved;\n");
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        let ty = match route.kind {
            RetainedRouteKind::Latest => "wl_latest_t",
            RetainedRouteKind::Fifo => "wl_fifo_t",
        };
        writeln!(output, "  {ty} *{message}_{kind};").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        output.push_str("  wl_rpc_client_t *rpc_client;\n  wl_rpc_server_t *rpc_server;\n");
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(output, "  {module}_{service_name}_rpc_t {service_name};").unwrap();
        }
    }
    writeln!(output, "}} {module}_runtime_t;\n").unwrap();
    output.push_str(concat!(
        "/* Terminal consumer for RX events: with non-null ctx/event every RX\n",
        " * outcome releases the event exactly once. Do not chain another dispatcher\n",
        " * or release it again. Non-RX events are never released; matching RPC TX\n",
        " * terminal events advance the client but the caller still owns wl_tx_take(). */\n",
    ));
    writeln!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime, wl_time_ms_t now_ms);\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        emit_rpc_header_functions(&mut output, module, service);
    }
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

fn emit_rpc_header_types(output: &mut String, module: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* The decoded request and borrowed fields are valid only for the callback.\n * Return zero after copying anything needed asynchronously. A nonzero return\n * abandons the pending operation without manufacturing a response. */\ntypedef int32_t (*{module}_{service_name}_request_handler_fn)(void *user_data, const {request}_t *request, wl_delivery_t delivery);\ntypedef struct {{\n  {request}_t *request_scratch;\n  {response}_t *response_scratch;\n  {module}_encode_scratch_t canonical_request_scratch;\n  {module}_{service_name}_request_handler_fn request_handler;\n  void *user_data;\n}} {module}_{service_name}_rpc_t;\n\n"
    )
    .unwrap();
}

fn emit_rpc_header_functions(output: &mut String, module: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* Client start writes the allocated operation ID into request in place. */\n{module}_runtime_result_t {module}_{service_name}_client_start_scratch(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms, {module}_encode_scratch_t scratch);\n{module}_runtime_result_t {module}_{service_name}_client_start_direct(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms);\n\n/* Completion writes operation ID and status into response in place, encodes\n * once, caches those bytes, and sends the exact cached byte sequence. */\n{module}_runtime_result_t {module}_{service_name}_server_complete(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint32_t operation_id, {response}_t *response, {module}_encode_scratch_t scratch, wl_time_ms_t now_ms);\n{module}_runtime_result_t {module}_{service_name}_server_reject(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint32_t operation_id, int32_t application_status, {response}_t *response, {module}_encode_scratch_t scratch, wl_time_ms_t now_ms);\n/* cached.response_data remains owned by wl_rpc_server_t; send or copy it before\n * the next server mutation, poll, or expiry. */\n{module}_runtime_result_t {module}_{service_name}_server_retry_cached(wl_ctx_t *ctx, const wl_rpc_server_response_t *cached);\n\n"
    )
    .unwrap();
}

fn emit_source(profile: &BindingProfileModel, module: &str) -> String {
    let prefix = upper_snake(module);
    let mut output = format!(
        "#include \"{module}_runtime.h\"\n\nstatic {module}_runtime_result_t {module}_runtime_result(const wl_event_t *event) {{\n  {module}_runtime_result_t result = {{0}};\n  result.domain = {prefix}_RUNTIME_INVALID_ARGUMENT;\n  result.codec_status = WL_CODEC_OK;\n  result.storage_result = WL_OK;\n  result.abort_result = WL_OK;\n  result.rpc_result = WL_RPC_OK;\n  result.core_result = WL_OK;\n  result.rpc_disposition = WL_RPC_SERVER_NEW;\n  if (event != NULL) {{\n    result.message_id = event->message_id;\n    result.event_type = event->type;\n  }}\n  return result;\n}}\n\n"
    );
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "static uint64_t {module}_rpc_request_fingerprint(const uint8_t *data, size_t length) {{\n  static const uint8_t domain[] = \"wlc.rpc.canonical-request.v1\";\n  uint64_t hash = UINT64_C(0xcbf29ce484222325);\n  size_t index;\n  for (index = 0U; index + 1U < sizeof(domain); ++index) {{\n    hash ^= (uint64_t)domain[index];\n    hash *= UINT64_C(0x00000100000001b3);\n  }}\n  hash ^= UINT64_C(0xff);\n  hash *= UINT64_C(0x00000100000001b3);\n  for (index = 0U; index < length; ++index) {{\n    hash ^= (uint64_t)data[index];\n    hash *= UINT64_C(0x00000100000001b3);\n  }}\n  return hash;\n}}\n\n"
        )
        .unwrap();
    }
    write!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime, wl_time_ms_t now_ms) {{\n  {module}_runtime_result_t result = {module}_runtime_result(event);\n  if (event == NULL) return result;\n"
    )
    .unwrap();
    output.push_str("  (void)now_ms;\n");
    if profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n"
        )
        .unwrap();
    } else {
        write!(
            output,
            "  if (event->type == WL_EVT_TX_SUCCESS || event->type == WL_EVT_TX_TIMEOUT || event->type == WL_EVT_TX_FAILED) {{\n    if (runtime == NULL || runtime->rpc_client == NULL) {{\n      result.domain = {prefix}_RUNTIME_NON_RX;\n      return result;\n    }}\n    result.rpc_result = wl_rpc_client_on_tx_event(runtime->rpc_client, event);\n    if (result.rpc_result == WL_RPC_OK) result.domain = {prefix}_RUNTIME_OK;\n    else if (result.rpc_result == WL_RPC_ERR_NOT_FOUND) result.domain = {prefix}_RUNTIME_NON_RX;\n    else result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n"
        )
        .unwrap();
    }
    output.push_str("  if (ctx == NULL) return result;\n  if (runtime == NULL) goto release_event;\n\n  switch (event->message_id) {\n");
    for route in &profile.retained_routes {
        emit_retained_case(&mut output, module, &prefix, route);
    }
    for service in &profile.rpc_services {
        emit_rpc_request_case(&mut output, module, &prefix, service);
        emit_rpc_response_case(&mut output, module, &prefix, service);
    }
    write!(
        output,
        "    default:\n      result.domain = {prefix}_RUNTIME_UNKNOWN_MESSAGE;\n      break;\n  }}\n\nrelease_event:\n  wl_event_release(ctx, event);\n  return result;\n}}\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        output.push('\n');
        emit_rpc_implementation(&mut output, module, &prefix, service);
    }
    output
}

fn delivery_event(delivery: DeliveryPolicy) -> &'static str {
    match delivery {
        DeliveryPolicy::Unreliable => "WL_EVT_UNRELIABLE_RX",
        DeliveryPolicy::Reliable => "WL_EVT_RELIABLE_RX",
    }
}

fn delivery_value(delivery: DeliveryPolicy) -> &'static str {
    match delivery {
        DeliveryPolicy::Unreliable => "WL_DELIVERY_UNRELIABLE",
        DeliveryPolicy::Reliable => "WL_DELIVERY_RELIABLE",
    }
}

fn delivery_suffix(delivery: DeliveryPolicy) -> &'static str {
    match delivery {
        DeliveryPolicy::Unreliable => "unreliable",
        DeliveryPolicy::Reliable => "reliable",
    }
}

fn emit_rpc_request_case(output: &mut String, module: &str, prefix: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let request_macro = upper_snake(&service.request_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.name);
    let expected_event = delivery_event(service.request_delivery);
    let delivery = delivery_value(service.request_delivery);
    let now_ms = "now_ms";
    write!(
        output,
        "    case {request_macro}_MESSAGE_ID: {{\n      wl_rpc_request_identity_t identity = {{0}};\n      wl_rpc_server_response_t replay = {{0}};\n      {module}_runtime_result_t retry;\n      size_t canonical_length = 0U;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_server == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.request_scratch == NULL || runtime->{service_name}.canonical_request_scratch.data == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.codec_status = {request}_decode(event->payload, event->payload_len, runtime->{service_name}.request_scratch);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.request_scratch->has_{operation_field} || runtime->{service_name}.request_scratch->{operation_field} == 0U) {{\n        result.rpc_result = WL_RPC_ERR_INVALID_ARG;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.operation_id = runtime->{service_name}.request_scratch->{operation_field};\n      result.codec_status = {request}_encode(runtime->{service_name}.request_scratch, runtime->{service_name}.canonical_request_scratch.data, runtime->{service_name}.canonical_request_scratch.capacity, &canonical_length);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.payload_length = canonical_length;\n      identity.operation_id = result.operation_id;\n      identity.request_message_id = {request_macro}_MESSAGE_ID;\n      identity.response_message_id = {response_macro}_MESSAGE_ID;\n      identity.request_fingerprint = {module}_rpc_request_fingerprint(runtime->{service_name}.canonical_request_scratch.data, canonical_length);\n      result.rpc_result = wl_rpc_server_begin(runtime->rpc_server, &identity, {now_ms}, &result.rpc_disposition, &replay);\n      if (result.rpc_result != WL_RPC_OK) {{\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.server_response = replay;\n      switch (result.rpc_disposition) {{\n        case WL_RPC_SERVER_NEW:\n          if (runtime->{service_name}.request_handler == NULL) {{\n            result.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, result.operation_id);\n            result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n            break;\n          }}\n          result.application_result = runtime->{service_name}.request_handler(runtime->{service_name}.user_data, runtime->{service_name}.request_scratch, {delivery});\n          if (result.application_result != 0) {{\n            result.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, result.operation_id);\n            result.domain = {prefix}_RUNTIME_APPLICATION_ERROR;\n          }} else {{\n            result.domain = {prefix}_RUNTIME_OK;\n          }}\n          break;\n        case WL_RPC_SERVER_PENDING_DUPLICATE:\n          result.domain = {prefix}_RUNTIME_OK;\n          break;\n        case WL_RPC_SERVER_REPLAY:\n          retry = {module}_{service_name}_server_retry_cached(ctx, &replay);\n          result.core_result = retry.core_result;\n          result.handle = retry.handle;\n          result.payload_length = retry.payload_length;\n          result.application_result = retry.application_result;\n          result.domain = retry.domain;\n          break;\n        case WL_RPC_SERVER_CONFLICT:\n          result.rpc_result = WL_RPC_ERR_OPERATION_CONFLICT;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n        default:\n          result.rpc_result = WL_RPC_ERR_INVALID_STATE;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n      }}\n      break;\n    }}\n"
    )
    .unwrap();
}

fn emit_rpc_response_case(output: &mut String, module: &str, prefix: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.response_operation_id.name);
    let status_field = c_identifier(&service.response_status.name);
    let expected_event = delivery_event(service.response_delivery);
    write!(
        output,
        "    case {response_macro}_MESSAGE_ID: {{\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_client == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.response_scratch == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.codec_status = {response}_decode(event->payload, event->payload_len, runtime->{service_name}.response_scratch);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.response_scratch->has_{operation_field} || runtime->{service_name}.response_scratch->{operation_field} == 0U || !runtime->{service_name}.response_scratch->has_{status_field}) {{\n        result.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.operation_id = runtime->{service_name}.response_scratch->{operation_field};\n      result.application_result = (int32_t)runtime->{service_name}.response_scratch->{status_field};\n      result.payload_length = event->payload_len;\n      result.rpc_result = wl_rpc_client_on_response(runtime->rpc_client, {response_macro}_MESSAGE_ID, result.operation_id, result.application_result, event->payload, event->payload_len);\n      result.domain = result.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n      break;\n    }}\n"
    )
    .unwrap();
    let _ = module;
}

fn emit_rpc_implementation(output: &mut String, module: &str, prefix: &str, service: &RpcService) {
    emit_rpc_client_implementation(output, module, prefix, service);
    emit_rpc_server_implementation(output, module, prefix, service);
}

fn emit_rpc_client_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    service: &RpcService,
) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let request_macro = upper_snake(&service.request_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.name);
    let delivery = delivery_value(service.request_delivery);
    let send_suffix = delivery_suffix(service.request_delivery);
    let transition = match service.request_delivery {
        DeliveryPolicy::Unreliable => {
            "wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)"
        }
        DeliveryPolicy::Reliable => {
            "wl_rpc_client_bind_tx(runtime->rpc_client, operation_id, result.handle)"
        }
    };
    write!(
        output,
        "static {module}_runtime_result_t {module}_{service_name}_client_finish_start({module}_runtime_t *runtime, uint32_t operation_id, {module}_send_result_t sent) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {request_macro}_MESSAGE_ID;\n  result.operation_id = operation_id;\n  result.codec_status = sent.codec_status;\n  result.core_result = sent.core_result;\n  result.abort_result = sent.abort_result;\n  result.handle = sent.handle;\n  result.payload_length = sent.payload_length;\n  if (sent.domain == {prefix}_SEND_CODEC_ERROR) {{\n    result.rpc_result = wl_rpc_client_link_failed(runtime->rpc_client, operation_id, WL_ERR_CORRUPT_PAYLOAD);\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  if (sent.domain == {prefix}_SEND_CORE_ERROR) {{\n    result.rpc_result = wl_rpc_client_link_failed(runtime->rpc_client, operation_id, sent.core_result);\n    result.domain = {prefix}_RUNTIME_CORE_ERROR;\n    return result;\n  }}\n  result.rpc_result = {transition};\n  result.domain = result.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n  return result;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_start_scratch(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms, {module}_encode_scratch_t scratch) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  {module}_send_result_t sent;\n  uint32_t operation_id = 0U;\n  result.message_id = {request_macro}_MESSAGE_ID;\n  if (ctx == NULL || runtime == NULL || runtime->rpc_client == NULL || request == NULL) return result;\n  result.rpc_result = wl_rpc_client_begin(runtime->rpc_client, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms, &operation_id);\n  result.operation_id = operation_id;\n  if (result.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  request->has_{operation_field} = true;\n  request->{operation_field} = operation_id;\n  sent = {module}_{request}_send_{send_suffix}(ctx, request, scratch);\n  return {module}_{service_name}_client_finish_start(runtime, operation_id, sent);\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_start_direct(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  {module}_send_result_t sent;\n  uint32_t operation_id = 0U;\n  result.message_id = {request_macro}_MESSAGE_ID;\n  if (ctx == NULL || runtime == NULL || runtime->rpc_client == NULL || request == NULL) return result;\n  result.rpc_result = wl_rpc_client_begin(runtime->rpc_client, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms, &operation_id);\n  result.operation_id = operation_id;\n  if (result.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  request->has_{operation_field} = true;\n  request->{operation_field} = operation_id;\n  sent = {module}_{request}_send_direct(ctx, request, {delivery});\n  return {module}_{service_name}_client_finish_start(runtime, operation_id, sent);\n}}\n\n"
    )
    .unwrap();
}

fn emit_rpc_server_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    service: &RpcService,
) {
    let service_name = c_identifier(&service.name);
    let request_macro = upper_snake(&service.request_name);
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.response_operation_id.name);
    let status_field = c_identifier(&service.response_status.name);
    let send_call = match service.response_delivery {
        DeliveryPolicy::Unreliable => format!(
            "wl_send_unreliable(ctx, {response_macro}_MESSAGE_ID, cached->response_data, cached->response_length)"
        ),
        DeliveryPolicy::Reliable => format!(
            "wl_send_reliable(ctx, {response_macro}_MESSAGE_ID, cached->response_data, cached->response_length, &result.handle)"
        ),
    };
    write!(
        output,
        "{module}_runtime_result_t {module}_{service_name}_server_retry_cached(wl_ctx_t *ctx, const wl_rpc_server_response_t *cached) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {response_macro}_MESSAGE_ID;\n  if (ctx == NULL || cached == NULL || cached->identity.operation_id == 0U || cached->identity.request_message_id != {request_macro}_MESSAGE_ID || cached->identity.response_message_id != {response_macro}_MESSAGE_ID || (cached->response_length != 0U && cached->response_data == NULL)) return result;\n  result.operation_id = cached->identity.operation_id;\n  result.application_result = cached->application_status;\n  result.payload_length = cached->response_length;\n  result.server_response = *cached;\n  result.core_result = {send_call};\n  result.domain = result.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n  return result;\n}}\n\nstatic {module}_runtime_result_t {module}_{service_name}_server_finish(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint32_t operation_id, int32_t application_status, {response}_t *response, {module}_encode_scratch_t scratch, wl_time_ms_t now_ms, bool reject) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  wl_rpc_server_response_t cached = {{0}};\n  size_t encoded_length = 0U;\n  result.message_id = {response_macro}_MESSAGE_ID;\n  result.operation_id = operation_id;\n  result.application_result = application_status;\n  if (ctx == NULL || runtime == NULL || runtime->rpc_server == NULL || operation_id == 0U || response == NULL) return result;\n  if (reject && application_status == 0) {{\n    result.rpc_result = WL_RPC_ERR_INVALID_ARG;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  response->has_{operation_field} = true;\n  response->{operation_field} = operation_id;\n  response->has_{status_field} = true;\n  response->{status_field} = application_status;\n  result.codec_status = {response}_encode(response, scratch.data, scratch.capacity, &encoded_length);\n  result.payload_length = encoded_length;\n  if (result.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  if (reject) result.rpc_result = wl_rpc_server_reject(runtime->rpc_server, operation_id, application_status, scratch.data, encoded_length, now_ms, &cached);\n  else result.rpc_result = wl_rpc_server_complete(runtime->rpc_server, operation_id, application_status, scratch.data, encoded_length, now_ms, &cached);\n  if (result.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  return {module}_{service_name}_server_retry_cached(ctx, &cached);\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_complete(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint32_t operation_id, {response}_t *response, {module}_encode_scratch_t scratch, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(ctx, runtime, operation_id, 0, response, scratch, now_ms, false);\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_reject(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint32_t operation_id, int32_t application_status, {response}_t *response, {module}_encode_scratch_t scratch, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(ctx, runtime, operation_id, application_status, response, scratch, now_ms, true);\n}}\n"
    )
    .unwrap();
}

fn emit_retained_case(
    output: &mut String,
    module: &str,
    prefix: &str,
    route: &crate::profile_semantic::RetainedRoute,
) {
    let message = type_name(&route.message_name);
    let message_macro = upper_snake(&route.message_name);
    let expected_event = match route.delivery {
        DeliveryPolicy::Unreliable => "WL_EVT_UNRELIABLE_RX",
        DeliveryPolicy::Reliable => "WL_EVT_RELIABLE_RX",
    };
    match route.kind {
        RetainedRouteKind::Latest => {
            write!(
                output,
                "    case {message_macro}_MESSAGE_ID: {{\n      wl_latest_write_claim_t claim = {{0}};\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->{message}_latest == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      result.storage_result = wl_latest_write_claim(runtime->{message}_latest, &claim);\n      if (result.storage_result != WL_OK) {{\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (claim.value_size < sizeof({message}_t)) {{\n        result.storage_result = WL_ERR_BUF_TOO_SMALL;\n        result.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (((uintptr_t)claim.value % _Alignof({message}_t)) != 0U) {{\n        result.storage_result = WL_ERR_INVALID_ARG;\n        result.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.codec_status = {message}_decode(event->payload, event->payload_len, ({message}_t *)claim.value);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.storage_result = wl_latest_write_publish(runtime->{message}_latest, &claim);\n      if (result.storage_result != WL_OK) {{\n        result.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.domain = {prefix}_RUNTIME_OK;\n      break;\n    }}\n"
            )
            .unwrap();
        }
        RetainedRouteKind::Fifo => {
            write!(
                output,
                "    case {message_macro}_MESSAGE_ID: {{\n      wl_fifo_write_claim_t claim = {{0}};\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->{message}_fifo == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      result.storage_result = wl_fifo_write_claim(runtime->{message}_fifo, &claim);\n      if (result.storage_result != WL_OK) {{\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (claim.value_size < sizeof({message}_t)) {{\n        result.storage_result = WL_ERR_BUF_TOO_SMALL;\n        result.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (((uintptr_t)claim.value % _Alignof({message}_t)) != 0U) {{\n        result.storage_result = WL_ERR_INVALID_ARG;\n        result.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.codec_status = {message}_decode(event->payload, event->payload_len, ({message}_t *)claim.value);\n      if (result.codec_status != WL_CODEC_OK) {{\n        result.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.storage_result = wl_fifo_write_publish(runtime->{message}_fifo, &claim);\n      if (result.storage_result != WL_OK) {{\n        result.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.domain = {prefix}_RUNTIME_OK;\n      break;\n    }}\n"
            )
            .unwrap();
        }
    }
    let _ = module;
}
