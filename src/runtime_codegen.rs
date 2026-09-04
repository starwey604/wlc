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
    manifest::CODEGEN_ABI_VERSION,
    profile::BINDING_PROFILE_VERSION,
    profile_semantic::{
        BindingProfileModel, DeliveryPolicy, RetainedRoute, RetainedRouteKind, RpcService,
        RpcStatusDomain,
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
        format!("{module}_runtime_detail_kind_t"),
        format!("{module}_runtime_detail_t"),
        format!("{module}_runtime_t"),
        format!("{module}_runtime_config_t"),
        format!("{module}_runtime_requirements_t"),
        format!("{module}_runtime_storage_t"),
        format!("{module}_runtime_instance_t"),
        format!("{module}_runtime_storage_cursor_t"),
        format!("{module}_runtime_layout_t"),
        format!("{module}_runtime_storage_region"),
        format!("{module}_runtime_layout"),
        format!("{module}_runtime_requirements"),
        format!("{module}_runtime_init"),
        format!("{module}_runtime_dispatch_event"),
        format!("{module}_runtime_result"),
        format!("WIRELINK_GENERATED_{prefix}_RUNTIME_H"),
        format!("{prefix}_SCHEMA_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_VERSION"),
        format!("{prefix}_IDENTITY_ALGORITHM"),
        format!("{prefix}_RUNTIME_CODEGEN_ABI_VERSION"),
        format!("{prefix}_RUNTIME_DETAIL_NONE"),
        format!("{prefix}_RUNTIME_DETAIL_RETAINED"),
        format!("{prefix}_RUNTIME_DETAIL_RPC"),
    ]);
    if !profile.retained_routes.is_empty() {
        runtime_names.insert(format!("{module}_runtime_retained_detail_t"));
    }
    if !profile.rpc_services.is_empty() {
        for symbol in [
            format!("{module}_runtime_rpc_detail_t"),
            format!("{module}_runtime_poll_result_t"),
            format!("{module}_runtime_service_result_t"),
            format!("{module}_runtime_poll"),
            format!("{module}_runtime_service"),
            format!("{module}_runtime_send_response"),
            format!("{module}_runtime_get_deadline_hint"),
            format!("{module}_rpc_request_fingerprint"),
            format!("{prefix}_RPC_REQUEST_FINGERPRINT_ALGORITHM"),
        ] {
            runtime_names.insert(symbol);
        }
    }
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        for symbol in [
            format!("{module}_{message}_{kind}_view_t"),
            format!("{module}_{message}_{kind}_acquire"),
            format!("{module}_{message}_{kind}_release"),
        ] {
            if !runtime_names.insert(symbol.clone()) {
                return Err(RuntimeCodegenError(format!(
                    "generated runtime symbols collide as C identifier `{symbol}`"
                )));
            }
        }
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
            format!("{module}_{name}_rpc_request_handler_fn"),
            format!("{module}_{name}_rpc_t"),
            format!("{module}_{name}_client_start"),
            format!("{module}_{name}_client_finish_start"),
            format!("{module}_{name}_client_inspect"),
            format!("{module}_{name}_client_decode"),
            format!("{module}_{name}_client_release"),
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
    if !matches!(
        field.cardinality,
        Cardinality::Optional | Cardinality::Required
    ) || field.ty != ResolvedType::Uint32
    {
        return Err(RuntimeCodegenError(format!(
            "RPC operation field `{}.{name}` must remain optional or required uint32",
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
    if !matches!(
        field.cardinality,
        Cardinality::Optional | Cardinality::Required
    ) {
        return Err(RuntimeCodegenError(format!(
            "RPC status field `{}.{}` must remain optional or required",
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
    let mut output =
        format!("#ifndef {guard}\n#define {guard}\n\n#include \"{module}_bindings.h\"\n");
    if profile
        .retained_routes
        .iter()
        .any(|route| route.kind == RetainedRouteKind::Fifo)
    {
        output.push_str("#include <wirelink/fifo.h>\n");
    }
    if profile
        .retained_routes
        .iter()
        .any(|route| route.kind == RetainedRouteKind::Latest)
    {
        output.push_str("#include <wirelink/latest.h>\n");
    }
    if !profile.rpc_services.is_empty() {
        output.push_str("#include <wirelink/rpc.h>\n");
    }
    output.push_str("\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
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
    writeln!(
        output,
        "#define {prefix}_RUNTIME_CODEGEN_ABI_VERSION {CODEGEN_ABI_VERSION}U\n"
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
        "typedef uint8_t {module}_runtime_detail_kind_t;\nenum {{\n  {prefix}_RUNTIME_DETAIL_NONE = 0,\n  {prefix}_RUNTIME_DETAIL_RETAINED = 1,\n  {prefix}_RUNTIME_DETAIL_RPC = 2\n}};\n\n"
    )
    .unwrap();
    if !profile.retained_routes.is_empty() {
        write!(
            output,
            "typedef struct {{\n  wl_codec_status_t codec_status;\n  int32_t storage_result;\n  int32_t abort_result;\n}} {module}_runtime_retained_detail_t;\n\n"
        )
        .unwrap();
    }
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "typedef struct {{\n  wl_codec_status_t codec_status;\n  wl_rpc_err_t rpc_result;\n  int32_t core_result;\n  int32_t application_result;\n  wl_rpc_server_disposition_t rpc_disposition;\n  uint32_t operation_id;\n  wl_tx_handle_t handle;\n  size_t payload_length;\n  union {{\n    wl_rpc_server_request_t server_request;\n    wl_rpc_server_response_t server_response;\n  }};\n}} {module}_runtime_rpc_detail_t;\n\n"
        )
        .unwrap();
    }
    output.push_str("typedef union {\n");
    if !profile.retained_routes.is_empty() {
        writeln!(output, "  {module}_runtime_retained_detail_t retained;").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        writeln!(output, "  {module}_runtime_rpc_detail_t rpc;").unwrap();
    }
    write!(
        output,
        "}} {module}_runtime_detail_t;\n\n/* Inspect detail only through the member selected by detail_kind. domain\n * classifies the outcome; zero-initialized unused detail fields retain their\n * corresponding success values. */\ntypedef struct {{\n  {module}_runtime_domain_t domain;\n  wl_event_type_t event_type;\n  uint16_t message_id;\n  {module}_runtime_detail_kind_t detail_kind;\n  uint8_t _reserved;\n  {module}_runtime_detail_t detail;\n}} {module}_runtime_result_t;\n\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        emit_retained_header_type(&mut output, module, route);
    }
    for service in &profile.rpc_services {
        emit_rpc_header_types(&mut output, module, service);
    }
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "typedef struct {{\n  uint16_t client_timed_out;\n  uint16_t server_pending_expired;\n  uint16_t server_cache_expired;\n  wl_rpc_server_request_t server_expired_request;\n}} {module}_runtime_poll_result_t;\n\ntypedef struct {{\n  {module}_runtime_poll_result_t deadlines;\n  {module}_runtime_result_t response;\n  uint16_t responses_submitted;\n  uint16_t responses_deferred;\n}} {module}_runtime_service_result_t;\n\n"
        )
        .unwrap();
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
    emit_assembly_header(&mut output, profile, module);
    output.push_str(concat!(
        "/* Terminal consumer for RX events: with non-null ctx/event every RX\n",
        " * outcome releases the event exactly once. Do not chain another dispatcher\n",
        " * or release it again. Matching RPC TX terminal events advance the runtime\n",
        " * and reclaim the handle. Unmatched non-RX events remain caller-owned. */\n",
    ));
    writeln!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime, wl_time_ms_t now_ms);\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        emit_retained_header_functions(&mut output, module, route);
    }
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "/* Advance configured RPC deadlines without performing I/O. At most one\n * expired server identity is returned per call and remains pending until the\n * application completes, rejects, or abandons it. */\nwl_rpc_err_t {module}_runtime_poll({module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_poll_result_t *out_result);\n/* Advance deadlines and submit at most one runtime-owned server response.\n * Link backpressure defers the same cached bytes for a later service call. */\nwl_rpc_err_t {module}_runtime_service(wl_ctx_t *ctx, {module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_service_result_t *out_result);\n/* Side-effect free. Zero is due; WL_RPC_NO_DEADLINE_MS means no deadline. */\nwl_rpc_err_t {module}_runtime_get_deadline_hint(const {module}_runtime_t *runtime, wl_time_ms_t now_ms, wl_rpc_deadline_hint_t *out_hint);\n\n"
        )
        .unwrap();
    }
    for service in &profile.rpc_services {
        emit_rpc_header_functions(&mut output, module, service);
    }
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

fn emit_retained_header_type(output: &mut String, module: &str, route: &RetainedRoute) {
    let message = type_name(&route.message_name);
    match route.kind {
        RetainedRouteKind::Latest => {
            write!(
                output,
                "/* The typed value remains borrowed until the matching release. */\ntypedef struct {{\n  const {message}_t *value;\n  uint32_t generation;\n  wl_latest_view_t lease;\n}} {module}_{message}_latest_view_t;\n\n"
            )
            .unwrap();
        }
        RetainedRouteKind::Fifo => {
            write!(
                output,
                "/* The typed value remains borrowed until the matching release. */\ntypedef struct {{\n  const {message}_t *value;\n  wl_fifo_view_t lease;\n}} {module}_{message}_fifo_view_t;\n\n"
            )
            .unwrap();
        }
    }
}

fn emit_retained_header_functions(output: &mut String, module: &str, route: &RetainedRoute) {
    let message = type_name(&route.message_name);
    let kind = match route.kind {
        RetainedRouteKind::Latest => "latest",
        RetainedRouteKind::Fifo => "fifo",
    };
    write!(
        output,
        "int {module}_{message}_{kind}_acquire({module}_runtime_t *runtime, {module}_{message}_{kind}_view_t *out_view);\nint {module}_{message}_{kind}_release({module}_runtime_t *runtime, {module}_{message}_{kind}_view_t *view);\n\n"
    )
    .unwrap();
}

fn emit_assembly_header(output: &mut String, profile: &BindingProfileModel, module: &str) {
    output.push_str(concat!(
        "/* Static runtime assembly. requirements() validates every sizing field and\n",
        " * reports the exact caller-owned byte storage needed by init(). Configuration\n",
        " * and storage descriptors may be temporary; instance and storage must outlive\n",
        " * all runtime activity and must not be copied after successful initialization. */\n",
        "typedef struct {\n",
        "  uint8_t _reserved;\n",
    ));
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                writeln!(output, "  uint32_t {message}_latest_initial_generation;").unwrap();
            }
            RetainedRouteKind::Fifo => {
                writeln!(output, "  uint32_t {message}_fifo_capacity;").unwrap();
            }
        }
    }
    if !profile.rpc_services.is_empty() {
        output.push_str(concat!(
            "  uint8_t rpc_client_enabled;\n",
            "  uint16_t rpc_client_slot_count;\n",
            "  uint16_t rpc_client_response_capacity;\n",
            "  uint32_t rpc_client_next_operation_id;\n",
            "  uint8_t rpc_server_enabled;\n",
            "  uint16_t rpc_server_pending_slot_count;\n",
            "  uint16_t rpc_server_cache_slot_count;\n",
            "  uint16_t rpc_server_response_capacity;\n",
            "  uint32_t rpc_server_pending_timeout_ms;\n",
            "  uint32_t rpc_server_cache_ttl_ms;\n",
            "  wl_rpc_cache_policy_t rpc_server_cache_policy;\n",
        ));
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(
                output,
                "  size_t {service_name}_canonical_request_capacity;"
            )
            .unwrap();
            writeln!(
                output,
                "  {module}_{service_name}_rpc_request_handler_fn {service_name}_request_handler;"
            )
            .unwrap();
            writeln!(output, "  void *{service_name}_user_data;").unwrap();
        }
    }
    writeln!(output, "}} {module}_runtime_config_t;\n").unwrap();
    write!(
        output,
        "typedef struct {{\n  size_t storage_size;\n  size_t storage_alignment;\n}} {module}_runtime_requirements_t;\n\ntypedef struct {{\n  void *data;\n  size_t size;\n}} {module}_runtime_storage_t;\n\ntypedef struct {{\n  {module}_runtime_t runtime;\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let (ty, kind) = match route.kind {
            RetainedRouteKind::Latest => ("wl_latest_t", "latest"),
            RetainedRouteKind::Fifo => ("wl_fifo_t", "fifo"),
        };
        writeln!(output, "  {ty} {message}_{kind};").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        output.push_str("  wl_rpc_client_t rpc_client;\n  wl_rpc_server_t rpc_server;\n");
        output.push_str(
            "  /* Dispatch is serialized; request and response decode scratch lifetimes do not overlap. */\n",
        );
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            let request = type_name(&service.request_name);
            let response = type_name(&service.response_name);
            writeln!(
                output,
                "  union {{ {request}_t request; {response}_t response; }} {service_name}_scratch;"
            )
            .unwrap();
        }
    }
    writeln!(
        output,
        "}} {module}_runtime_instance_t;\n\nint {module}_runtime_requirements(const {module}_runtime_config_t *config, {module}_runtime_requirements_t *out_requirements);\nint {module}_runtime_init({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage);\n"
    )
    .unwrap();
}

fn emit_rpc_header_types(output: &mut String, module: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* The decoded request and its borrowed fields are valid only for the\n * callback. Copy server_request for asynchronous completion. Its generation\n * prevents a late completion from targeting a reused request identity. A\n * nonzero return abandons this exact pending operation. */\ntypedef int32_t (*{module}_{service_name}_rpc_request_handler_fn)(void *user_data, const {request}_t *request, const wl_rpc_server_request_t *server_request, wl_delivery_t delivery);\ntypedef struct {{\n  {request}_t *request_scratch;\n  {response}_t *response_scratch;\n  {module}_encode_scratch_t canonical_request_scratch;\n  {module}_{service_name}_rpc_request_handler_fn request_handler;\n  void *user_data;\n}} {module}_{service_name}_rpc_t;\n\n"
    )
    .unwrap();
}

fn emit_rpc_header_functions(output: &mut String, module: &str, service: &RpcService) {
    let service_name = c_identifier(&service.name);
    let request = type_name(&service.request_name);
    let response = type_name(&service.response_name);
    write!(
        output,
        "/* Allocates, encodes, and submits atomically from the caller's view. A\n * present nonzero request operation ID is used exactly, allowing an explicit\n * retry to address the server's bounded replay cache; absent or zero selects an\n * automatically allocated ID. The request is restored before return. A local\n * encode/submit failure releases the allocated RPC slot and returns operation_id\n * zero. */\n{module}_runtime_result_t {module}_{service_name}_client_start(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms);\n/* Nonblocking inspection returns generic metadata for this service. */\nwl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client);\n/* Decode a retained response previously returned by client_inspect(). Borrowed\n * response fields remain valid only until client_release(). */\n{module}_runtime_result_t {module}_{service_name}_client_decode(const wl_rpc_client_result_t *client, {response}_t *response);\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id);\n\n/* server_request is copied from the request callback and uniquely scopes this\n * execution generation. Completion encodes directly into the response storage\n * reserved by wl_rpc_server_begin(); runtime_service() performs I/O and restores\n * the caller-owned response before return. */\n{module}_runtime_result_t {module}_{service_name}_server_complete({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, {response}_t *response, wl_time_ms_t now_ms);\n{module}_runtime_result_t {module}_{service_name}_server_reject({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, {response}_t *response, wl_time_ms_t now_ms);\n\n"
    )
    .unwrap();
}

fn emit_assembly_source(output: &mut String, profile: &BindingProfileModel, module: &str) {
    write!(
        output,
        "typedef struct {{\n  uint8_t *base;\n  size_t size;\n  size_t offset;\n}} {module}_runtime_storage_cursor_t;\n\ntypedef struct {{\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        writeln!(output, "  void *{message}_{kind}_storage;").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        output.push_str(concat!(
            "  void *rpc_client_slots;\n",
            "  void *rpc_client_responses;\n",
            "  size_t rpc_client_responses_size;\n",
            "  void *rpc_server_pending_slots;\n",
            "  void *rpc_server_cache_slots;\n",
            "  void *rpc_server_responses;\n",
            "  size_t rpc_server_responses_size;\n",
        ));
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(output, "  void *{service_name}_canonical_request_storage;").unwrap();
        }
    }
    write!(
        output,
        "}} {module}_runtime_layout_t;\n\nstatic int {module}_runtime_storage_region({module}_runtime_storage_cursor_t *cursor, size_t alignment, size_t count, size_t element_size, void **out_data, size_t *out_size) {{\n  size_t aligned;\n  size_t region_size;\n  if (cursor == NULL || alignment == 0U || (alignment & (alignment - 1U)) != 0U) return WL_ERR_INVALID_ARG;\n  if (out_data != NULL) *out_data = NULL;\n  if (out_size != NULL) *out_size = 0U;\n  if (count != 0U && element_size > SIZE_MAX / count) return WL_ERR_INVALID_ARG;\n  region_size = count * element_size;\n  if (cursor->offset > SIZE_MAX - (alignment - 1U)) return WL_ERR_INVALID_ARG;\n  aligned = (cursor->offset + (alignment - 1U)) & ~(alignment - 1U);\n  if (region_size > SIZE_MAX - aligned) return WL_ERR_INVALID_ARG;\n  if (aligned + region_size > cursor->size) return WL_ERR_BUF_TOO_SMALL;\n  if (out_data != NULL && cursor->base != NULL) *out_data = cursor->base + aligned;\n  if (out_size != NULL) *out_size = region_size;\n  cursor->offset = aligned + region_size;\n  return WL_OK;\n}}\n\nstatic int {module}_runtime_layout(const {module}_runtime_config_t *config, uint8_t *base, size_t size, {module}_runtime_layout_t *out_layout, {module}_runtime_requirements_t *out_requirements) {{\n  {module}_runtime_storage_cursor_t cursor = {{base, size, 0U}};\n  size_t alignment = 1U;\n  int result;\n  if (out_layout != NULL) memset(out_layout, 0, sizeof(*out_layout));\n  if (out_requirements != NULL) memset(out_requirements, 0, sizeof(*out_requirements));\n  if (config == NULL) return WL_ERR_INVALID_ARG;\n"
    )
    .unwrap();

    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                write!(
                    output,
                    "  {{\n    const wl_latest_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_latest_initial_generation}};\n    wl_latest_requirements_t route_requirements;\n    result = wl_latest_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) return result;\n    if (alignment < _Alignof({message}_t)) alignment = _Alignof({message}_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof({message}_t), 1U, route_requirements.storage_size, out_layout == NULL ? NULL : &out_layout->{message}_latest_storage, NULL);\n    if (result != WL_OK) return result;\n  }}\n"
                )
                .unwrap();
            }
            RetainedRouteKind::Fifo => {
                write!(
                    output,
                    "  {{\n    const wl_fifo_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_fifo_capacity}};\n    wl_fifo_requirements_t route_requirements;\n    result = wl_fifo_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) return result;\n    if (alignment < _Alignof({message}_t)) alignment = _Alignof({message}_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof({message}_t), 1U, route_requirements.storage_size, out_layout == NULL ? NULL : &out_layout->{message}_fifo_storage, NULL);\n    if (result != WL_OK) return result;\n  }}\n"
                )
                .unwrap();
            }
        }
    }

    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (config->rpc_client_enabled > 1U || config->rpc_server_enabled > 1U) return WL_ERR_INVALID_ARG;\n  if (config->rpc_client_enabled != 0U) {{\n    if (config->rpc_client_slot_count == 0U || config->rpc_client_response_capacity == 0U) return WL_ERR_INVALID_ARG;\n    if (alignment < _Alignof(wl_rpc_client_slot_t)) alignment = _Alignof(wl_rpc_client_slot_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_client_slot_t), config->rpc_client_slot_count, sizeof(wl_rpc_client_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_client_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, 1U, config->rpc_client_slot_count, config->rpc_client_response_capacity, out_layout == NULL ? NULL : &out_layout->rpc_client_responses, out_layout == NULL ? NULL : &out_layout->rpc_client_responses_size);\n    if (result != WL_OK) return result;\n  }}\n  if (config->rpc_server_enabled != 0U) {{\n    if (config->rpc_server_pending_slot_count == 0U || config->rpc_server_cache_slot_count == 0U || config->rpc_server_response_capacity == 0U) return WL_ERR_INVALID_ARG;\n    if ((config->rpc_server_pending_timeout_ms != 0U && config->rpc_server_pending_timeout_ms >= UINT32_C(0x80000000)) || (config->rpc_server_cache_ttl_ms != 0U && config->rpc_server_cache_ttl_ms >= UINT32_C(0x80000000))) return WL_ERR_INVALID_ARG;\n    if (config->rpc_server_cache_policy != WL_RPC_CACHE_REJECT_NEW && config->rpc_server_cache_policy != WL_RPC_CACHE_EVICT_OLDEST) return WL_ERR_INVALID_ARG;\n    if (alignment < _Alignof(wl_rpc_server_pending_slot_t)) alignment = _Alignof(wl_rpc_server_pending_slot_t);\n    if (alignment < _Alignof(wl_rpc_server_cache_slot_t)) alignment = _Alignof(wl_rpc_server_cache_slot_t);\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_server_pending_slot_t), config->rpc_server_pending_slot_count, sizeof(wl_rpc_server_pending_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_server_pending_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, _Alignof(wl_rpc_server_cache_slot_t), config->rpc_server_cache_slot_count, sizeof(wl_rpc_server_cache_slot_t), out_layout == NULL ? NULL : &out_layout->rpc_server_cache_slots, NULL);\n    if (result != WL_OK) return result;\n    result = {module}_runtime_storage_region(&cursor, 1U, config->rpc_server_cache_slot_count, config->rpc_server_response_capacity, out_layout == NULL ? NULL : &out_layout->rpc_server_responses, out_layout == NULL ? NULL : &out_layout->rpc_server_responses_size);\n    if (result != WL_OK) return result;\n"
        )
        .unwrap();
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            write!(
                output,
                "    if (config->{service_name}_canonical_request_capacity == 0U) return WL_ERR_INVALID_ARG;\n    result = {module}_runtime_storage_region(&cursor, 1U, 1U, config->{service_name}_canonical_request_capacity, out_layout == NULL ? NULL : &out_layout->{service_name}_canonical_request_storage, NULL);\n    if (result != WL_OK) return result;\n"
            )
            .unwrap();
        }
        output.push_str("  }\n");
    }
    write!(
        output,
        "  if (out_requirements != NULL) {{\n    out_requirements->storage_size = cursor.offset;\n    out_requirements->storage_alignment = alignment;\n  }}\n  return WL_OK;\n}}\n\nint {module}_runtime_requirements(const {module}_runtime_config_t *config, {module}_runtime_requirements_t *out_requirements) {{\n  {module}_runtime_config_t config_copy;\n  if (config == NULL || out_requirements == NULL) return WL_ERR_INVALID_ARG;\n  config_copy = *config;\n  *out_requirements = ({module}_runtime_requirements_t){{0}};\n  return {module}_runtime_layout(&config_copy, NULL, SIZE_MAX, NULL, out_requirements);\n}}\n\nint {module}_runtime_init({module}_runtime_instance_t *instance, const {module}_runtime_config_t *config, const {module}_runtime_storage_t *storage) {{\n  {module}_runtime_config_t config_copy;\n  {module}_runtime_storage_t storage_copy;\n  {module}_runtime_requirements_t requirements;\n  {module}_runtime_layout_t layout;\n  uintptr_t instance_address;\n  uintptr_t storage_address;\n  int result;\n  if (instance == NULL || config == NULL || storage == NULL) return WL_ERR_INVALID_ARG;\n  config_copy = *config;\n  storage_copy = *storage;\n  config = &config_copy;\n  storage = &storage_copy;\n  result = {module}_runtime_requirements(config, &requirements);\n  if (result != WL_OK) return result;\n  if (storage->size < requirements.storage_size) return WL_ERR_BUF_TOO_SMALL;\n  if (requirements.storage_size != 0U) {{\n    if (storage->data == NULL || ((uintptr_t)storage->data & (requirements.storage_alignment - 1U)) != 0U) return WL_ERR_INVALID_ARG;\n    instance_address = (uintptr_t)(void *)instance;\n    storage_address = (uintptr_t)storage->data;\n    if ((storage_address <= instance_address && instance_address - storage_address < requirements.storage_size) || (instance_address < storage_address && storage_address - instance_address < sizeof(*instance))) return WL_ERR_INVALID_ARG;\n  }}\n  result = {module}_runtime_layout(config, (uint8_t *)storage->data, storage->size, &layout, NULL);\n  if (result != WL_OK) return result;\n  memset(instance, 0, sizeof(*instance));\n"
    )
    .unwrap();

    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        match route.kind {
            RetainedRouteKind::Latest => {
                write!(
                    output,
                    "  {{\n    const wl_latest_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_latest_initial_generation}};\n    wl_latest_requirements_t route_requirements;\n    wl_latest_storage_t route_storage;\n    result = wl_latest_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) goto init_failed;\n    route_storage.data = layout.{message}_latest_storage;\n    route_storage.size = route_requirements.storage_size;\n    result = wl_latest_init(&instance->{message}_latest, &route_config, &route_storage);\n    if (result != WL_OK) goto init_failed;\n    instance->runtime.{message}_latest = &instance->{message}_latest;\n  }}\n"
                )
                .unwrap();
            }
            RetainedRouteKind::Fifo => {
                write!(
                    output,
                    "  {{\n    const wl_fifo_config_t route_config = {{sizeof({message}_t), _Alignof({message}_t), config->{message}_fifo_capacity}};\n    wl_fifo_requirements_t route_requirements;\n    wl_fifo_storage_t route_storage;\n    result = wl_fifo_requirements(&route_config, &route_requirements);\n    if (result != WL_OK) goto init_failed;\n    route_storage.data = layout.{message}_fifo_storage;\n    route_storage.size = route_requirements.storage_size;\n    result = wl_fifo_init(&instance->{message}_fifo, &route_config, &route_storage);\n    if (result != WL_OK) goto init_failed;\n    instance->runtime.{message}_fifo = &instance->{message}_fifo;\n  }}\n"
                )
                .unwrap();
            }
        }
    }

    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  if (config->rpc_client_enabled != 0U) {{\n    const wl_rpc_client_config_t client_config = {{\n      (wl_rpc_client_slot_t *)layout.rpc_client_slots,\n      config->rpc_client_slot_count,\n      (uint8_t *)layout.rpc_client_responses,\n      layout.rpc_client_responses_size,\n      config->rpc_client_response_capacity,\n      config->rpc_client_next_operation_id\n    }};\n    if (wl_rpc_client_init(&instance->rpc_client, &client_config) != WL_RPC_OK) {{\n      result = WL_ERR_INVALID_ARG;\n      goto init_failed;\n    }}\n    instance->runtime.rpc_client = &instance->rpc_client;\n  }}\n  if (config->rpc_server_enabled != 0U) {{\n    const wl_rpc_server_config_t server_config = {{\n      (wl_rpc_server_pending_slot_t *)layout.rpc_server_pending_slots,\n      config->rpc_server_pending_slot_count,\n      (wl_rpc_server_cache_slot_t *)layout.rpc_server_cache_slots,\n      config->rpc_server_cache_slot_count,\n      (uint8_t *)layout.rpc_server_responses,\n      layout.rpc_server_responses_size,\n      config->rpc_server_response_capacity,\n      config->rpc_server_pending_timeout_ms,\n      config->rpc_server_cache_ttl_ms,\n      config->rpc_server_cache_policy\n    }};\n    if (wl_rpc_server_init(&instance->rpc_server, &server_config) != WL_RPC_OK) {{\n      result = WL_ERR_INVALID_ARG;\n      goto init_failed;\n    }}\n    instance->runtime.rpc_server = &instance->rpc_server;\n  }}\n"
        )
        .unwrap();
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            write!(
                output,
                "  if (config->rpc_server_enabled != 0U) {{\n    instance->runtime.{service_name}.request_scratch = &instance->{service_name}_scratch.request;\n    instance->runtime.{service_name}.canonical_request_scratch.data = (uint8_t *)layout.{service_name}_canonical_request_storage;\n    instance->runtime.{service_name}.canonical_request_scratch.capacity = config->{service_name}_canonical_request_capacity;\n    instance->runtime.{service_name}.request_handler = config->{service_name}_request_handler;\n    instance->runtime.{service_name}.user_data = config->{service_name}_user_data;\n  }}\n  if (config->rpc_client_enabled != 0U) instance->runtime.{service_name}.response_scratch = &instance->{service_name}_scratch.response;\n"
            )
            .unwrap();
        }
    }
    output.push_str(concat!(
        "  return WL_OK;\n",
        "\n",
        "init_failed:\n",
        "  memset(instance, 0, sizeof(*instance));\n",
        "  return result;\n",
        "}\n\n",
    ));
}

fn emit_source(profile: &BindingProfileModel, module: &str) -> String {
    let prefix = upper_snake(module);
    let mut output = format!(
        "#include \"{module}_runtime.h\"\n\n#include <string.h>\n\nstatic {module}_runtime_result_t {module}_runtime_result(const wl_event_t *event) {{\n  {module}_runtime_result_t result = {{0}};\n  result.domain = {prefix}_RUNTIME_INVALID_ARGUMENT;\n  if (event != NULL) {{\n    result.message_id = event->message_id;\n    result.event_type = event->type;\n  }}\n  return result;\n}}\n\n"
    );
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "static uint64_t {module}_rpc_request_fingerprint(const uint8_t *data, size_t length) {{\n  static const uint8_t domain[] = \"wlc.rpc.canonical-request.v1\";\n  uint64_t hash = UINT64_C(0xcbf29ce484222325);\n  size_t index;\n  for (index = 0U; index + 1U < sizeof(domain); ++index) {{\n    hash ^= (uint64_t)domain[index];\n    hash *= UINT64_C(0x00000100000001b3);\n  }}\n  hash ^= UINT64_C(0xff);\n  hash *= UINT64_C(0x00000100000001b3);\n  for (index = 0U; index < length; ++index) {{\n    hash ^= (uint64_t)data[index];\n    hash *= UINT64_C(0x00000100000001b3);\n  }}\n  return hash;\n}}\n\n"
        )
        .unwrap();
    }
    emit_assembly_source(&mut output, profile, module);
    if !profile.rpc_services.is_empty() {
        emit_rpc_runtime_progress_implementation(&mut output, module, &prefix, profile);
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
            "  if (event->type == WL_EVT_TX_SUCCESS || event->type == WL_EVT_TX_TIMEOUT || event->type == WL_EVT_TX_FAILED) {{\n    wl_tx_result_t tx_result = {{0}};\n    if (runtime == NULL || ctx == NULL) {{\n      result.domain = {prefix}_RUNTIME_NON_RX;\n      return result;\n    }}\n    result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n    result.detail.rpc.handle = event->handle;\n    if (runtime->rpc_server != NULL) {{\n      result.detail.rpc.rpc_result = wl_rpc_server_on_tx_event(runtime->rpc_server, event);\n      if (result.detail.rpc.rpc_result == WL_RPC_OK) {{\n        result.detail.rpc.core_result = wl_tx_take(ctx, event->handle, &tx_result);\n        result.domain = result.detail.rpc.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n        return result;\n      }}\n      if (result.detail.rpc.rpc_result != WL_RPC_ERR_NOT_FOUND) {{\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        return result;\n      }}\n    }}\n    if (runtime->rpc_client != NULL) {{\n      result.detail.rpc.rpc_result = wl_rpc_client_on_tx_event(runtime->rpc_client, event);\n      if (result.detail.rpc.rpc_result == WL_RPC_OK) {{\n        result.detail.rpc.core_result = wl_tx_take(ctx, event->handle, &tx_result);\n        result.domain = result.detail.rpc.core_result == WL_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_CORE_ERROR;\n      }} else if (result.detail.rpc.rpc_result == WL_RPC_ERR_NOT_FOUND) result.domain = {prefix}_RUNTIME_NON_RX;\n      else result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    }} else {{\n      result.domain = {prefix}_RUNTIME_NON_RX;\n    }}\n    return result;\n  }}\n  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n"
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
    for route in &profile.retained_routes {
        output.push('\n');
        emit_retained_implementation(&mut output, module, route);
    }
    for service in &profile.rpc_services {
        output.push('\n');
        emit_rpc_implementation(&mut output, module, &prefix, service);
    }
    output
}

fn emit_rpc_runtime_progress_implementation(
    output: &mut String,
    module: &str,
    prefix: &str,
    profile: &BindingProfileModel,
) {
    write!(
        output,
        "wl_rpc_err_t {module}_runtime_poll({module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_poll_result_t *out_result) {{\n  wl_rpc_err_t result;\n  wl_rpc_server_expiry_t server_expiry = {{0}};\n  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_poll(runtime->rpc_client, now_ms, &out_result->client_timed_out);\n    if (result != WL_RPC_OK) return result;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = wl_rpc_server_expired_acquire(runtime->rpc_server, now_ms, &out_result->server_expired_request);\n    if (result == WL_RPC_OK) out_result->server_pending_expired = 1U;\n    else if (result != WL_RPC_ERR_NOT_FOUND) return result;\n    result = wl_rpc_server_poll(runtime->rpc_server, now_ms, &server_expiry);\n    if (result != WL_RPC_OK) return result;\n    out_result->server_cache_expired = server_expiry.cache_expired;\n  }}\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_service(wl_ctx_t *ctx, {module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_service_result_t *out_result) {{\n  wl_rpc_server_response_t response = {{0}};\n  wl_rpc_err_t result;\n  uint8_t reliable_response = 0U;\n  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));\n  if (ctx == NULL || runtime == NULL || out_result == NULL) return WL_RPC_ERR_INVALID_ARG;\n  out_result->response = {module}_runtime_result(NULL);\n  result = {module}_runtime_poll(runtime, now_ms, &out_result->deadlines);\n  if (result != WL_RPC_OK) return result;\n  if (runtime->rpc_server == NULL) return WL_RPC_OK;\n  result = wl_rpc_server_response_acquire(runtime->rpc_server, &response);\n  if (result == WL_RPC_ERR_NOT_FOUND) return WL_RPC_OK;\n  if (result != WL_RPC_OK) return result;\n  out_result->response.message_id = response.identity.response_message_id;\n  out_result->response.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  out_result->response.detail.rpc.operation_id = response.identity.operation_id;\n  out_result->response.detail.rpc.application_result = response.application_status;\n  out_result->response.detail.rpc.payload_length = response.response_length;\n  out_result->response.detail.rpc.server_response = response;\n  switch (response.identity.response_message_id) {{\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        let request_macro = upper_snake(&service.request_name);
        let response_macro = upper_snake(&service.response_name);
        let reliable = match service.response_delivery {
            DeliveryPolicy::Unreliable => "0U",
            DeliveryPolicy::Reliable => "1U",
        };
        writeln!(
            output,
            "    case {response_macro}_MESSAGE_ID:\n      if (response.identity.request_message_id != {request_macro}_MESSAGE_ID) {{\n        result = WL_RPC_ERR_RESPONSE_MISMATCH;\n        break;\n      }}\n      reliable_response = {reliable};\n      break;"
        )
        .unwrap();
    }
    write!(
        output,
        "    default:\n      result = WL_RPC_ERR_RESPONSE_MISMATCH;\n      break;\n  }}\n  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  if (reliable_response != 0U) {{\n    out_result->response.detail.rpc.core_result = wl_send_reliable(ctx, response.identity.response_message_id, response.response_data, response.response_length, &out_result->response.detail.rpc.handle);\n  }} else {{\n    out_result->response.detail.rpc.core_result = wl_send_unreliable(ctx, response.identity.response_message_id, response.response_data, response.response_length);\n  }}\n  if (out_result->response.detail.rpc.core_result != WL_OK) {{\n    result = wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    if (result != WL_RPC_OK) return result;\n    out_result->response.domain = {prefix}_RUNTIME_CORE_ERROR;\n    out_result->responses_deferred = 1U;\n    return WL_RPC_OK;\n  }}\n  if (reliable_response != 0U) {{\n    result = wl_rpc_server_response_submitted(runtime->rpc_server, &response, out_result->response.detail.rpc.handle);\n  }} else {{\n    result = wl_rpc_server_response_sent(runtime->rpc_server, &response);\n  }}\n"
    )
    .unwrap();
    write!(
        output,
        "  if (result != WL_RPC_OK) {{\n    (void)wl_rpc_server_response_defer(runtime->rpc_server, &response);\n    return result;\n  }}\n  out_result->response.domain = {prefix}_RUNTIME_OK;\n  out_result->responses_submitted = 1U;\n  return WL_RPC_OK;\n}}\n\nwl_rpc_err_t {module}_runtime_get_deadline_hint(const {module}_runtime_t *runtime, wl_time_ms_t now_ms, wl_rpc_deadline_hint_t *out_hint) {{\n  wl_rpc_deadline_hint_t component = {{WL_RPC_NO_DEADLINE_MS}};\n  wl_rpc_err_t result;\n  uint32_t nearest = WL_RPC_NO_DEADLINE_MS;\n  if (out_hint != NULL) out_hint->next_deadline_ms = WL_RPC_NO_DEADLINE_MS;\n  if (runtime == NULL || out_hint == NULL) return WL_RPC_ERR_INVALID_ARG;\n  if (runtime->rpc_client != NULL) {{\n    result = wl_rpc_client_get_deadline_hint(runtime->rpc_client, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  if (runtime->rpc_server != NULL) {{\n    result = wl_rpc_server_get_deadline_hint(runtime->rpc_server, now_ms, &component);\n    if (result != WL_RPC_OK) return result;\n    if (component.next_deadline_ms < nearest) nearest = component.next_deadline_ms;\n  }}\n  out_hint->next_deadline_ms = nearest;\n  return WL_RPC_OK;\n}}\n\n"
    )
    .unwrap();
}

fn emit_retained_implementation(output: &mut String, module: &str, route: &RetainedRoute) {
    let message = type_name(&route.message_name);
    match route.kind {
        RetainedRouteKind::Latest => {
            write!(
                output,
                "int {module}_{message}_latest_acquire({module}_runtime_t *runtime, {module}_{message}_latest_view_t *out_view) {{\n  wl_latest_view_t lease = {{0}};\n  int result;\n  if (out_view != NULL) memset(out_view, 0, sizeof(*out_view));\n  if (runtime == NULL || out_view == NULL) return WL_ERR_INVALID_ARG;\n  if (runtime->{message}_latest == NULL) return WL_ERR_NOT_INITIALIZED;\n  result = wl_latest_read_acquire(runtime->{message}_latest, &lease);\n  if (result != WL_OK) return result;\n  if (lease.value == NULL || lease.value_size < sizeof({message}_t) || ((uintptr_t)lease.value % _Alignof({message}_t)) != 0U) {{\n    int failure = lease.value_size < sizeof({message}_t) ? WL_ERR_BUF_TOO_SMALL : WL_ERR_INVALID_STATE;\n    int release_result = wl_latest_read_release(runtime->{message}_latest, &lease);\n    if (release_result != WL_OK) return release_result;\n    return failure;\n  }}\n  out_view->value = (const {message}_t *)lease.value;\n  out_view->generation = lease.generation;\n  out_view->lease = lease;\n  return WL_OK;\n}}\n\nint {module}_{message}_latest_release({module}_runtime_t *runtime, {module}_{message}_latest_view_t *view) {{\n  int result;\n  if (runtime == NULL || view == NULL) return WL_ERR_INVALID_ARG;\n  if (runtime->{message}_latest == NULL) return WL_ERR_NOT_INITIALIZED;\n  if ((const void *)view->value != view->lease.value || view->generation != view->lease.generation) return WL_ERR_INVALID_STATE;\n  result = wl_latest_read_release(runtime->{message}_latest, &view->lease);\n  if (result == WL_OK) memset(view, 0, sizeof(*view));\n  return result;\n}}\n"
            )
            .unwrap();
        }
        RetainedRouteKind::Fifo => {
            write!(
                output,
                "int {module}_{message}_fifo_acquire({module}_runtime_t *runtime, {module}_{message}_fifo_view_t *out_view) {{\n  wl_fifo_view_t lease = {{0}};\n  int result;\n  if (out_view != NULL) memset(out_view, 0, sizeof(*out_view));\n  if (runtime == NULL || out_view == NULL) return WL_ERR_INVALID_ARG;\n  if (runtime->{message}_fifo == NULL) return WL_ERR_NOT_INITIALIZED;\n  result = wl_fifo_read_acquire(runtime->{message}_fifo, &lease);\n  if (result != WL_OK) return result;\n  if (lease.value == NULL || lease.value_size < sizeof({message}_t) || ((uintptr_t)lease.value % _Alignof({message}_t)) != 0U) {{\n    int failure = lease.value_size < sizeof({message}_t) ? WL_ERR_BUF_TOO_SMALL : WL_ERR_INVALID_STATE;\n    int release_result = wl_fifo_read_release(runtime->{message}_fifo, &lease);\n    if (release_result != WL_OK) return release_result;\n    return failure;\n  }}\n  out_view->value = (const {message}_t *)lease.value;\n  out_view->lease = lease;\n  return WL_OK;\n}}\n\nint {module}_{message}_fifo_release({module}_runtime_t *runtime, {module}_{message}_fifo_view_t *view) {{\n  int result;\n  if (runtime == NULL || view == NULL) return WL_ERR_INVALID_ARG;\n  if (runtime->{message}_fifo == NULL) return WL_ERR_NOT_INITIALIZED;\n  if ((const void *)view->value != view->lease.value) return WL_ERR_INVALID_STATE;\n  result = wl_fifo_read_release(runtime->{message}_fifo, &view->lease);\n  if (result == WL_OK) memset(view, 0, sizeof(*view));\n  return result;\n}}\n"
            )
            .unwrap();
        }
    }
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
        "    case {request_macro}_MESSAGE_ID: {{\n      wl_rpc_request_identity_t identity = {{0}};\n      wl_rpc_server_request_t server_request = {{0}};\n      wl_rpc_server_response_t replay = {{0}};\n      size_t canonical_length = 0U;\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_server == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.request_scratch == NULL || runtime->{service_name}.canonical_request_scratch.data == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.detail.rpc.codec_status = {request}_decode(event->payload, event->payload_len, runtime->{service_name}.request_scratch);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.request_scratch->has_{operation_field} || runtime->{service_name}.request_scratch->{operation_field} == 0U) {{\n        result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_ARG;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.detail.rpc.operation_id = runtime->{service_name}.request_scratch->{operation_field};\n      result.detail.rpc.codec_status = {request}_encode(runtime->{service_name}.request_scratch, runtime->{service_name}.canonical_request_scratch.data, runtime->{service_name}.canonical_request_scratch.capacity, &canonical_length);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.detail.rpc.payload_length = canonical_length;\n      identity.operation_id = result.detail.rpc.operation_id;\n      identity.request_message_id = {request_macro}_MESSAGE_ID;\n      identity.response_message_id = {response_macro}_MESSAGE_ID;\n      identity.request_fingerprint = {module}_rpc_request_fingerprint(runtime->{service_name}.canonical_request_scratch.data, canonical_length);\n      identity.peer_session_id = event->peer_session_id;\n      result.detail.rpc.rpc_result = wl_rpc_server_begin(runtime->rpc_server, &identity, {now_ms}, &result.detail.rpc.rpc_disposition, &server_request, &replay);\n      if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      switch (result.detail.rpc.rpc_disposition) {{\n        case WL_RPC_SERVER_NEW:\n          result.detail.rpc.server_request = server_request;\n          if (runtime->{service_name}.request_handler == NULL) {{\n            result.detail.rpc.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, &server_request);\n            result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n            break;\n          }}\n          result.detail.rpc.application_result = runtime->{service_name}.request_handler(runtime->{service_name}.user_data, runtime->{service_name}.request_scratch, &server_request, {delivery});\n          if (result.detail.rpc.application_result != 0) {{\n            result.detail.rpc.rpc_result = wl_rpc_server_abandon(runtime->rpc_server, &server_request);\n            result.domain = {prefix}_RUNTIME_APPLICATION_ERROR;\n          }} else {{\n            result.domain = {prefix}_RUNTIME_OK;\n          }}\n          break;\n        case WL_RPC_SERVER_PENDING_DUPLICATE:\n          result.domain = {prefix}_RUNTIME_OK;\n          break;\n        case WL_RPC_SERVER_REPLAY:\n          result.detail.rpc.server_response = replay;\n          result.detail.rpc.application_result = replay.application_status;\n          result.detail.rpc.payload_length = replay.response_length;\n          result.detail.rpc.core_result = WL_OK;\n          result.domain = {prefix}_RUNTIME_OK;\n          break;\n        case WL_RPC_SERVER_CONFLICT:\n          result.detail.rpc.rpc_result = WL_RPC_ERR_OPERATION_CONFLICT;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n        default:\n          result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_STATE;\n          result.domain = {prefix}_RUNTIME_RPC_ERROR;\n          break;\n      }}\n      break;\n    }}\n"
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
        "    case {response_macro}_MESSAGE_ID: {{\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->rpc_client == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      if (runtime->{service_name}.response_scratch == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_SCRATCH;\n        break;\n      }}\n      result.detail.rpc.codec_status = {response}_decode(event->payload, event->payload_len, runtime->{service_name}.response_scratch);\n      if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      if (!runtime->{service_name}.response_scratch->has_{operation_field} || runtime->{service_name}.response_scratch->{operation_field} == 0U || !runtime->{service_name}.response_scratch->has_{status_field}) {{\n        result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n        result.domain = {prefix}_RUNTIME_RPC_ERROR;\n        break;\n      }}\n      result.detail.rpc.operation_id = runtime->{service_name}.response_scratch->{operation_field};\n      result.detail.rpc.application_result = (int32_t)runtime->{service_name}.response_scratch->{status_field};\n      result.detail.rpc.payload_length = event->payload_len;\n      result.detail.rpc.rpc_result = wl_rpc_client_on_response(runtime->rpc_client, {response_macro}_MESSAGE_ID, result.detail.rpc.operation_id, result.detail.rpc.application_result, event->payload, event->payload_len);\n      result.domain = result.detail.rpc.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n      break;\n    }}\n"
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
    let response = type_name(&service.response_name);
    let response_macro = upper_snake(&service.response_name);
    let operation_field = c_identifier(&service.request_operation_id.name);
    let response_operation_field = c_identifier(&service.response_operation_id.name);
    let status_field = c_identifier(&service.response_status.name);
    let delivery = delivery_value(service.request_delivery);
    let transition = match service.request_delivery {
        DeliveryPolicy::Unreliable => {
            "wl_rpc_client_tx_completed(runtime->rpc_client, operation_id)"
        }
        DeliveryPolicy::Reliable => {
            "wl_rpc_client_bind_tx(runtime->rpc_client, operation_id, result.detail.rpc.handle)"
        }
    };
    write!(
        output,
        "static {module}_runtime_result_t {module}_{service_name}_client_finish_start({module}_runtime_t *runtime, uint32_t operation_id, {module}_send_result_t sent) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {request_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  result.detail.rpc.operation_id = operation_id;\n  result.detail.rpc.codec_status = sent.codec_status;\n  result.detail.rpc.core_result = sent.core_result;\n  result.detail.rpc.handle = sent.handle;\n  result.detail.rpc.payload_length = sent.payload_length;\n  if (sent.domain == {prefix}_SEND_CODEC_ERROR || sent.domain == {prefix}_SEND_CORE_ERROR) {{\n    const int32_t link_result = sent.domain == {prefix}_SEND_CODEC_ERROR ? WL_ERR_CORRUPT_PAYLOAD : sent.core_result;\n    result.detail.rpc.rpc_result = wl_rpc_client_link_failed(runtime->rpc_client, operation_id, link_result);\n    if (result.detail.rpc.rpc_result == WL_RPC_OK)\n      result.detail.rpc.rpc_result = wl_rpc_client_release(runtime->rpc_client, operation_id);\n    if (result.detail.rpc.rpc_result == WL_RPC_OK) result.detail.rpc.operation_id = 0U;\n    result.domain = sent.domain == {prefix}_SEND_CODEC_ERROR ? {prefix}_RUNTIME_CODEC_ERROR : {prefix}_RUNTIME_CORE_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = {transition};\n  result.domain = result.detail.rpc.rpc_result == WL_RPC_OK ? {prefix}_RUNTIME_OK : {prefix}_RUNTIME_RPC_ERROR;\n  return result;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_start(wl_ctx_t *ctx, {module}_runtime_t *runtime, {request}_t *request, uint32_t timeout_ms, wl_time_ms_t now_ms) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  {module}_send_result_t sent;\n  uint32_t operation_id = 0U;\n  bool had_operation_id;\n  uint32_t previous_operation_id;\n  result.message_id = {request_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  if (ctx == NULL || runtime == NULL || runtime->rpc_client == NULL || request == NULL) return result;\n  had_operation_id = request->has_{operation_field};\n  previous_operation_id = request->{operation_field};\n  if (had_operation_id && previous_operation_id != 0U) {{\n    operation_id = previous_operation_id;\n    result.detail.rpc.rpc_result = wl_rpc_client_begin_with_id(runtime->rpc_client, operation_id, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms);\n  }} else {{\n    result.detail.rpc.rpc_result = wl_rpc_client_begin(runtime->rpc_client, {request_macro}_MESSAGE_ID, {response_macro}_MESSAGE_ID, timeout_ms, now_ms, &operation_id);\n  }}\n  result.detail.rpc.operation_id = operation_id;\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  request->has_{operation_field} = true;\n  request->{operation_field} = operation_id;\n  sent = {module}_{request}_send(ctx, request, {delivery});\n  request->has_{operation_field} = had_operation_id;\n  request->{operation_field} = previous_operation_id;\n  return {module}_{service_name}_client_finish_start(runtime, operation_id, sent);\n}}\n\n"
    )
    .unwrap();
    write!(
        output,
        "wl_rpc_err_t {module}_{service_name}_client_inspect(const {module}_runtime_t *runtime, uint32_t operation_id, wl_rpc_client_result_t *out_client) {{\n  wl_rpc_err_t result;\n  if (out_client != NULL) memset(out_client, 0, sizeof(*out_client));\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U || out_client == NULL) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, out_client);\n  if (result != WL_RPC_OK) return result;\n  if (out_client->request_message_id != {request_macro}_MESSAGE_ID || out_client->response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return WL_RPC_OK;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_client_decode(const wl_rpc_client_result_t *client, {response}_t *response) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  result.message_id = {response_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  if (client != NULL) {{\n    result.detail.rpc.operation_id = client->operation_id;\n    result.detail.rpc.handle = client->tx_handle;\n    result.detail.rpc.core_result = client->link_result;\n    result.detail.rpc.application_result = client->application_status;\n    result.detail.rpc.payload_length = client->response_length;\n  }}\n  if (client == NULL || response == NULL || client->operation_id == 0U) return result;\n  {response}_clear(response);\n  if (client->request_message_id != {request_macro}_MESSAGE_ID || client->response_message_id != {response_macro}_MESSAGE_ID) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  if ((client->state != WL_RPC_CLIENT_COMPLETED && client->state != WL_RPC_CLIENT_APPLICATION_ERROR) || client->response_data == NULL || client->response_length == 0U) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_STATE;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.codec_status = {response}_decode(client->response_data, client->response_length, response);\n  if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  if (!response->has_{response_operation_field} || response->{response_operation_field} != client->operation_id || !response->has_{status_field} || (int32_t)response->{status_field} != client->application_status) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_RESPONSE_MISMATCH;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.domain = {prefix}_RUNTIME_OK;\n  return result;\n}}\n\nwl_rpc_err_t {module}_{service_name}_client_release({module}_runtime_t *runtime, uint32_t operation_id) {{\n  wl_rpc_client_result_t client = {{0}};\n  wl_rpc_err_t result;\n  if (runtime == NULL || runtime->rpc_client == NULL || operation_id == 0U) return WL_RPC_ERR_INVALID_ARG;\n  result = wl_rpc_client_get(runtime->rpc_client, operation_id, &client);\n  if (result != WL_RPC_OK) return result;\n  if (client.request_message_id != {request_macro}_MESSAGE_ID || client.response_message_id != {response_macro}_MESSAGE_ID) return WL_RPC_ERR_RESPONSE_MISMATCH;\n  return wl_rpc_client_release(runtime->rpc_client, operation_id);\n}}\n\n"
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
    write!(
        output,
        "static {module}_runtime_result_t {module}_{service_name}_server_finish({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, {response}_t *response, wl_time_ms_t now_ms, bool reject) {{\n  {module}_runtime_result_t result = {module}_runtime_result(NULL);\n  wl_rpc_server_response_buffer_t buffer = {{0}};\n  wl_rpc_server_response_t cached = {{0}};\n  size_t encoded_length = 0U;\n  bool had_operation_id;\n  uint32_t previous_operation_id;\n  bool had_status;\n  int32_t previous_status;\n  result.message_id = {response_macro}_MESSAGE_ID;\n  result.detail_kind = {prefix}_RUNTIME_DETAIL_RPC;\n  result.detail.rpc.application_result = application_status;\n  if (runtime == NULL || runtime->rpc_server == NULL || server_request == NULL || server_request->generation == 0U || server_request->identity.operation_id == 0U || server_request->identity.request_message_id != {request_macro}_MESSAGE_ID || server_request->identity.response_message_id != {response_macro}_MESSAGE_ID || response == NULL) return result;\n  result.detail.rpc.operation_id = server_request->identity.operation_id;\n  result.detail.rpc.server_request = *server_request;\n  if (reject && application_status == 0) {{\n    result.detail.rpc.rpc_result = WL_RPC_ERR_INVALID_ARG;\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = wl_rpc_server_response_prepare(runtime->rpc_server, server_request, &buffer);\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  had_operation_id = response->has_{operation_field};\n  previous_operation_id = response->{operation_field};\n  had_status = response->has_{status_field};\n  previous_status = (int32_t)response->{status_field};\n  response->has_{operation_field} = true;\n  response->{operation_field} = server_request->identity.operation_id;\n  response->has_{status_field} = true;\n  response->{status_field} = application_status;\n  result.detail.rpc.codec_status = {response}_encode(response, buffer.data, buffer.capacity, &encoded_length);\n  response->has_{operation_field} = had_operation_id;\n  response->{operation_field} = previous_operation_id;\n  response->has_{status_field} = had_status;\n  response->{status_field} = previous_status;\n  result.detail.rpc.payload_length = encoded_length;\n  if (result.detail.rpc.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.rpc_result = wl_rpc_server_response_commit(runtime->rpc_server, &buffer, application_status, encoded_length, now_ms, &cached);\n  if (result.detail.rpc.rpc_result != WL_RPC_OK) {{\n    result.domain = {prefix}_RUNTIME_RPC_ERROR;\n    return result;\n  }}\n  result.detail.rpc.server_response = cached;\n  result.detail.rpc.application_result = cached.application_status;\n  result.detail.rpc.payload_length = cached.response_length;\n  result.detail.rpc.core_result = WL_OK;\n  result.domain = {prefix}_RUNTIME_OK;\n  return result;\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_complete({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, {response}_t *response, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(runtime, server_request, 0, response, now_ms, false);\n}}\n\n{module}_runtime_result_t {module}_{service_name}_server_reject({module}_runtime_t *runtime, const wl_rpc_server_request_t *server_request, int32_t application_status, {response}_t *response, wl_time_ms_t now_ms) {{\n  return {module}_{service_name}_server_finish(runtime, server_request, application_status, response, now_ms, true);\n}}\n"
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
                "    case {message_macro}_MESSAGE_ID: {{\n      wl_latest_write_claim_t claim = {{0}};\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RETAINED;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->{message}_latest == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      result.detail.retained.storage_result = wl_latest_write_claim(runtime->{message}_latest, &claim);\n      if (result.detail.retained.storage_result != WL_OK) {{\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (claim.value_size < sizeof({message}_t)) {{\n        result.detail.retained.storage_result = WL_ERR_BUF_TOO_SMALL;\n        result.detail.retained.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (((uintptr_t)claim.value % _Alignof({message}_t)) != 0U) {{\n        result.detail.retained.storage_result = WL_ERR_INVALID_ARG;\n        result.detail.retained.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.detail.retained.codec_status = {message}_decode(event->payload, event->payload_len, ({message}_t *)claim.value);\n      if (result.detail.retained.codec_status != WL_CODEC_OK) {{\n        result.detail.retained.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.detail.retained.storage_result = wl_latest_write_publish(runtime->{message}_latest, &claim);\n      if (result.detail.retained.storage_result != WL_OK) {{\n        result.detail.retained.abort_result = wl_latest_write_abort(runtime->{message}_latest, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.domain = {prefix}_RUNTIME_OK;\n      break;\n    }}\n"
            )
            .unwrap();
        }
        RetainedRouteKind::Fifo => {
            write!(
                output,
                "    case {message_macro}_MESSAGE_ID: {{\n      wl_fifo_write_claim_t claim = {{0}};\n      result.detail_kind = {prefix}_RUNTIME_DETAIL_RETAINED;\n      if (event->type != {expected_event}) {{\n        result.domain = {prefix}_RUNTIME_DELIVERY_MISMATCH;\n        break;\n      }}\n      if (runtime->{message}_fifo == NULL) {{\n        result.domain = {prefix}_RUNTIME_MISSING_ROUTE;\n        break;\n      }}\n      result.detail.retained.storage_result = wl_fifo_write_claim(runtime->{message}_fifo, &claim);\n      if (result.detail.retained.storage_result != WL_OK) {{\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (claim.value_size < sizeof({message}_t)) {{\n        result.detail.retained.storage_result = WL_ERR_BUF_TOO_SMALL;\n        result.detail.retained.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      if (((uintptr_t)claim.value % _Alignof({message}_t)) != 0U) {{\n        result.detail.retained.storage_result = WL_ERR_INVALID_ARG;\n        result.detail.retained.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.detail.retained.codec_status = {message}_decode(event->payload, event->payload_len, ({message}_t *)claim.value);\n      if (result.detail.retained.codec_status != WL_CODEC_OK) {{\n        result.detail.retained.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_CODEC_ERROR;\n        break;\n      }}\n      result.detail.retained.storage_result = wl_fifo_write_publish(runtime->{message}_fifo, &claim);\n      if (result.detail.retained.storage_result != WL_OK) {{\n        result.detail.retained.abort_result = wl_fifo_write_abort(runtime->{message}_fifo, &claim);\n        result.domain = {prefix}_RUNTIME_STORAGE_ERROR;\n        break;\n      }}\n      result.domain = {prefix}_RUNTIME_OK;\n      break;\n    }}\n"
            )
            .unwrap();
        }
    }
    let _ = module;
}
