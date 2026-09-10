// SPDX-License-Identifier: Apache-2.0
use super::RuntimeCodegenError;
use crate::ast::Cardinality;
use crate::codegen::{c_identifier, type_name, upper_snake};
use crate::profile::BINDING_PROFILE_VERSION;
use crate::profile_semantic::{
    BindingProfileModel, RetainedRouteKind, RpcService, RpcStatusDomain,
};
use crate::semantic::{MessageSymbol, ResolvedType, SemanticModel, Symbol};
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) fn validate_runtime_names(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    module: &str,
) -> Result<(), RuntimeCodegenError> {
    let prefix = upper_snake(module);
    let mut member_names = BTreeSet::from([
        "_reserved".to_owned(),
        "rpc_client".to_owned(),
        "rpc_encode_scratch".to_owned(),
        "rpc_server".to_owned(),
        "rpc_retiring_tx".to_owned(),
    ]);
    if profile.rpc_services.iter().any(RpcService::is_managed) {
        member_names.insert("rpc_incarnation".to_owned());
        member_names.insert("rpc_async".to_owned());
    }
    for route in &profile.retained_routes {
        let kind = match route.kind {
            RetainedRouteKind::Latest => "latest",
            RetainedRouteKind::Fifo => "fifo",
        };
        member_names.insert(format!("{}_{kind}", type_name(&route.message_name)));
    }
    for route in &profile.direct_routes {
        member_names.insert(format!("{}_direct", type_name(&route.message_name)));
    }
    let mut handler_names = BTreeSet::from(["result".to_owned(), "response_terminal".to_owned()]);
    for name in profile
        .direct_routes
        .iter()
        .map(|route| type_name(&route.message_name))
        .chain(
            profile
                .rpc_services
                .iter()
                .map(|service| c_identifier(&service.name)),
        )
    {
        if !handler_names.insert(name.clone()) {
            return Err(RuntimeCodegenError(format!(
                "endpoint handlers collide as C identifier `on_{name}`"
            )));
        }
    }
    let mut runtime_names = BTreeSet::from([
        format!("{module}_endpoint_t"),
        format!("{module}_endpoint_config_t"),
        format!("{module}_runtime_domain_t"),
        format!("{module}_runtime_result_t"),
        format!("{module}_runtime_result_ok"),
        format!("{module}_runtime_result_str"),
        format!("{module}_runtime_detail_kind_t"),
        format!("{module}_runtime_detail_t"),
        format!("{module}_runtime_t"),
        format!("{module}_runtime_pump_t"),
        format!("{module}_runtime_result_fn"),
        format!("{module}_runtime_config_t"),
        format!("{module}_runtime_default_storage_alignment_t"),
        format!("{module}_runtime_default_storage_t"),
        format!("{module}_runtime_requirements_t"),
        format!("{module}_runtime_storage_t"),
        format!("{module}_runtime_instance_t"),
        format!("{module}_runtime_storage_cursor_t"),
        format!("{module}_runtime_layout_t"),
        format!("{module}_runtime_storage_region"),
        format!("{module}_runtime_layout"),
        format!("{module}_runtime_requirements"),
        format!("{module}_runtime_config_defaults"),
        format!("{module}_runtime_config_enable_client"),
        format!("{module}_runtime_config_enable_server"),
        format!("{module}_runtime_default_storage_descriptor"),
        format!("{module}_runtime_init"),
        format!("{module}_runtime_dispatch_event"),
        format!("{module}_runtime_pump_init"),
        format!("{module}_runtime_pump_hooks"),
        format!("{module}_runtime_pump_event"),
        format!("{module}_runtime_pump_progress"),
        format!("{module}_runtime_pump_deadline"),
        format!("{module}_runtime_result"),
        format!("WIRELINK_GENERATED_{prefix}_RUNTIME_H"),
        format!("{prefix}_ENDPOINT_H"),
        format!("{prefix}_ADVANCED_H"),
        format!("{prefix}_SCHEMA_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_IDENTITY"),
        format!("{prefix}_BINDING_PROFILE_VERSION"),
        format!("{prefix}_IDENTITY_ALGORITHM"),
        format!("{prefix}_RUNTIME_CODEGEN_ABI_VERSION"),
        format!("{prefix}_RUNTIME_HAS_DEFAULT_STORAGE"),
        format!("{prefix}_RUNTIME_DEFAULT_STORAGE_ALIGNMENT"),
        format!("{prefix}_RUNTIME_DEFAULT_STORAGE_CAPACITY"),
        format!("{prefix}_RUNTIME_DETAIL_NONE"),
        format!("{prefix}_RUNTIME_DETAIL_RETAINED"),
        format!("{prefix}_RUNTIME_DETAIL_RPC"),
    ]);
    if !profile.retained_routes.is_empty() {
        runtime_names.insert(format!("{module}_runtime_retained_detail_t"));
        runtime_names.insert(format!("{module}_runtime_result_retained_detail"));
    }
    if !profile.direct_routes.is_empty() {
        runtime_names.insert(format!("{module}_runtime_direct_detail_t"));
        runtime_names.insert(format!("{prefix}_RUNTIME_DETAIL_DIRECT"));
        for route in &profile.direct_routes {
            let name = type_name(&route.message_name);
            runtime_names.insert(format!("{module}_{name}_direct_fn"));
            runtime_names.insert(format!("{module}_{name}_direct_t"));
        }
    }
    if !profile.rpc_services.is_empty() {
        for symbol in [
            format!("{module}_runtime_rpc_detail_t"),
            format!("{module}_runtime_result_rpc_detail"),
            format!("{module}_runtime_rpc_encode_scratch_t"),
            format!("{module}_runtime_poll_result_t"),
            format!("{module}_runtime_service_result_t"),
            format!("{module}_runtime_poll"),
            format!("{module}_runtime_service"),
            format!("{module}_runtime_send_response"),
            format!("{module}_rpc_fingerprint_seed"),
            format!("{module}_runtime_get_deadline_hint"),
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
    if profile.rpc_services.iter().any(RpcService::is_managed) {
        for suffix in [
            "rpc_response_encoder_fn",
            "rpc_request_prepare",
            "rpc_request_begin",
            "rpc_finish_response",
        ] {
            runtime_names.insert(format!("{module}_{suffix}"));
        }
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

    if profile.rpc_services.iter().any(RpcService::is_managed) {
        for suffix in ["read_u32", "write_u32", "header_read", "header_write"] {
            runtime_names.insert(format!("{module}_rpc_{suffix}"));
        }
    }
    for service in profile
        .rpc_services
        .iter()
        .filter(|service| service.is_managed())
    {
        let name = c_identifier(&service.name);
        if name == "result" {
            return Err(RuntimeCodegenError(
                "endpoint handler field `on_result` is reserved for diagnostics".into(),
            ));
        }
        for suffix in [
            "request_token_t",
            "request_inspect",
            "call_t",
            "result_t",
            "handler_fn",
            "completion_fn",
            "server_complete_value",
            "encode_submission",
            "encode_response",
            "sync_state_t",
        ] {
            let symbol = format!("{module}_{name}_{suffix}");
            if !runtime_names.insert(symbol.clone()) {
                return Err(RuntimeCodegenError(format!(
                    "generated runtime symbols collide as C identifier `{symbol}`"
                )));
            }
        }
    }
    let mut schema_names = BTreeSet::new();
    for suffix in [
        "config_defaults",
        "handle",
        "runtime",
        "record",
        "init_config",
        "init",
        "step",
        "result",
        "close",
        "cancel",
        "driver",
        "driver_step",
        "driver_close",
        "create",
        "destroy",
    ] {
        runtime_names.insert(format!("{module}_endpoint_{suffix}"));
    }
    for suffix in [
        "MAX_PAYLOAD",
        "ALIGNMENT",
        "RAW_CAPACITY",
        "UNIT_CAPACITY",
        "CONTROL_CAPACITY",
        "RPC_CAPACITY",
        "REQUEST_CAPACITY",
        "RUNTIME_CAPACITY",
        "RX_FIFO_CAPACITY",
    ] {
        runtime_names.insert(format!("{prefix}_ENDPOINT_{suffix}"));
    }
    runtime_names.insert(format!("{prefix}_HAS_DEFAULT_ENDPOINT"));
    runtime_names.insert(format!("{prefix}_RUNTIME_HAS_RPC_CLIENT"));
    runtime_names.insert(format!("{prefix}_RUNTIME_HAS_RPC_SERVER"));
    runtime_names.insert(format!("{module}_runtime_roles_valid"));
    for route in &profile.retained_routes {
        let message = type_name(&route.message_name);
        for verb in ["send", "read"] {
            runtime_names.insert(format!("{module}_endpoint_{verb}_{message}"));
        }
    }
    for route in &profile.send_routes {
        runtime_names.insert(format!(
            "{module}_endpoint_send_{}",
            type_name(&route.message_name)
        ));
    }
    for service in &profile.rpc_services {
        let name = c_identifier(&service.name);
        runtime_names.insert(format!("{module}_runtime_{name}_decode_detail_t"));
        let verbs: &[&str] = if service.is_managed() {
            &[
                "call",
                "call_get",
                "record_result",
                "inspect",
                "release",
                "cancel",
                "complete",
                "reject",
                "async",
                "prepare",
                "notify",
                "submit_at",
                "sync",
                "sync_done",
                "proxy_submit",
            ]
        } else {
            &["start", "inspect", "release", "complete"]
        };
        for verb in verbs {
            let symbol = format!("{module}_endpoint_{name}_{verb}");
            if !runtime_names.insert(symbol.clone()) {
                return Err(RuntimeCodegenError(format!(
                    "generated endpoint symbols collide as C identifier `{symbol}`"
                )));
            }
        }
    }
    for symbol in &schema.declarations {
        let name = type_name(symbol.name());
        schema_names.insert(format!("{name}_t"));
        match symbol {
            Symbol::Message(message) => {
                schema_names.insert(format!("{name}_clear"));
                schema_names.insert(format!("{name}_encoded_size"));
                schema_names.insert(format!("{name}_encode"));
                schema_names.insert(format!("{name}_decode"));
                // Value symbols share the codec namespace, not a runtime prefix.
                // Reserving even an unavailable value also keeps names stable
                // when a schema later adds finite bounds.
                schema_names.insert(format!("{name}_value_t"));
                for verb in [
                    "clear",
                    "encoded_size",
                    "encode",
                    "decode",
                    "from_view",
                    "to_view",
                ] {
                    schema_names.insert(format!("{name}_value_{verb}"));
                }
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

pub(super) fn validate_profile_model(
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

    let mut send_ids = HashSet::new();
    for route in &profile.send_routes {
        let message = exact_message(&messages, &route.message_name, route.message_id)?;
        if !send_ids.insert(message.id) {
            return Err(RuntimeCodegenError(format!(
                "duplicate send binding for message `{}`",
                message.name
            )));
        }
    }
    let mut service_names = HashSet::new();
    let mut direct_ids = HashSet::new();
    for route in &profile.direct_routes {
        let message = exact_message(&messages, &route.message_name, route.message_id)?;
        if !direct_ids.insert(message.id) || retained_ids.contains(&message.id) {
            return Err(RuntimeCodegenError(format!(
                "message `{}` has multiple receive routes",
                message.name
            )));
        }
        if crate::profile_semantic::has_repeated_fields(message, &messages) {
            return Err(RuntimeCodegenError(format!(
                "direct message `{}` requires repeated backing",
                message.name
            )));
        }
    }
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
            if direct_ids.contains(&message.id) {
                return Err(RuntimeCodegenError(format!(
                    "RPC {role} message `{}` also has a direct route",
                    message.name
                )));
            }
            if send_ids.contains(&message.id) {
                return Err(RuntimeCodegenError(format!(
                    "RPC {role} message `{}` also has a plain send binding",
                    message.name
                )));
            }
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
        match (
            &service.request_operation_id,
            &service.response_operation_id,
            &service.response_status,
            &service.status_domain,
        ) {
            (None, None, None, None) => continue,
            (Some(_), Some(_), Some(_), Some(_)) => {}
            _ => {
                return Err(RuntimeCodegenError(
                    "RPC metadata mappings must be all present or all absent".to_owned(),
                ));
            }
        }
        validate_operation_field(
            request,
            &service.request_operation_id.as_ref().unwrap().name,
            service.request_operation_id.as_ref().unwrap().number,
        )?;
        validate_operation_field(
            response,
            &service.response_operation_id.as_ref().unwrap().name,
            service.response_operation_id.as_ref().unwrap().number,
        )?;
        validate_status_field(schema, response, service)?;
    }
    Ok(())
}

pub(super) fn exact_message<'a>(
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

pub(super) fn validate_operation_field(
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

pub(super) fn validate_status_field(
    schema: &SemanticModel,
    response: &MessageSymbol,
    service: &RpcService,
) -> Result<(), RuntimeCodegenError> {
    let mapping = service.response_status.as_ref().unwrap();
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
    match (&field.ty, service.status_domain.as_ref().unwrap()) {
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

pub(super) fn retained_ownership_problem(
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
