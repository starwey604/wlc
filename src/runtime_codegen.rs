//! Deterministic C runtime generation for an optional binding profile.

use std::{collections::BTreeSet, fmt::Write};

use miette::Diagnostic;
use thiserror::Error;

use crate::{
    codegen::{c_identifier, generate_c, type_name, upper_snake},
    identity::{IDENTITY_ALGORITHM, binding_profile_identity, schema_identity},
    profile_semantic::{BindingProfileModel, DeliveryPolicy, RetainedRouteKind},
    semantic::SemanticModel,
};

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
    validate_runtime_names(profile)?;

    Ok(GeneratedRuntimeC {
        header: emit_header(schema, profile, &module),
        source: emit_source(profile, &module),
    })
}

fn validate_runtime_names(profile: &BindingProfileModel) -> Result<(), RuntimeCodegenError> {
    let mut service_names = BTreeSet::new();
    for service in &profile.rpc_services {
        let name = c_identifier(&service.name);
        if name.is_empty() {
            return Err(RuntimeCodegenError(format!(
                "RPC service `{}` has no C identifier characters",
                service.name
            )));
        }
        if !service_names.insert(name.clone()) {
            return Err(RuntimeCodegenError(format!(
                "RPC services collide as C identifier `{name}`"
            )));
        }
    }
    Ok(())
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

    write!(
        output,
        "typedef int32_t {module}_runtime_domain_t;\nenum {{\n  {prefix}_RUNTIME_OK = 0,\n  {prefix}_RUNTIME_NON_RX,\n  {prefix}_RUNTIME_UNKNOWN_MESSAGE,\n  {prefix}_RUNTIME_MISSING_ROUTE,\n  {prefix}_RUNTIME_DELIVERY_MISMATCH,\n  {prefix}_RUNTIME_CODEC_ERROR,\n  {prefix}_RUNTIME_STORAGE_ERROR,\n  {prefix}_RUNTIME_RPC_ERROR,\n  {prefix}_RUNTIME_CORE_ERROR,\n  {prefix}_RUNTIME_APPLICATION_ERROR,\n  {prefix}_RUNTIME_INVALID_ARGUMENT\n}};\n\n"
    )
    .unwrap();
    write!(
        output,
        "typedef struct {{\n  {module}_runtime_domain_t domain;\n  uint16_t message_id;\n  wl_event_type_t event_type;\n  wl_codec_status_t codec_status;\n  int32_t storage_result;\n  int32_t abort_result;\n  wl_rpc_err_t rpc_result;\n  int32_t core_result;\n  int32_t application_result;\n  wl_rpc_server_disposition_t rpc_disposition;\n  uint32_t operation_id;\n  wl_tx_handle_t handle;\n  size_t payload_length;\n  wl_rpc_server_response_t server_response;\n}} {module}_runtime_result_t;\n\n"
    )
    .unwrap();
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
    writeln!(output, "}} {module}_runtime_t;\n").unwrap();
    writeln!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime);\n"
    )
    .unwrap();
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

fn emit_source(profile: &BindingProfileModel, module: &str) -> String {
    let prefix = upper_snake(module);
    let mut output = format!(
        "#include \"{module}_runtime.h\"\n\nstatic {module}_runtime_result_t {module}_runtime_result(const wl_event_t *event) {{\n  {module}_runtime_result_t result = {{0}};\n  result.domain = {prefix}_RUNTIME_INVALID_ARGUMENT;\n  result.codec_status = WL_CODEC_OK;\n  result.storage_result = WL_OK;\n  result.abort_result = WL_OK;\n  result.rpc_result = WL_RPC_OK;\n  result.core_result = WL_OK;\n  result.rpc_disposition = WL_RPC_SERVER_NEW;\n  if (event != NULL) {{\n    result.message_id = event->message_id;\n    result.event_type = event->type;\n  }}\n  return result;\n}}\n\n"
    );
    write!(
        output,
        "{module}_runtime_result_t {module}_runtime_dispatch_event(wl_ctx_t *ctx, const wl_event_t *event, {module}_runtime_t *runtime) {{\n  {module}_runtime_result_t result = {module}_runtime_result(event);\n  if (event == NULL) return result;\n  if (event->type != WL_EVT_UNRELIABLE_RX && event->type != WL_EVT_RELIABLE_RX) {{\n    result.domain = {prefix}_RUNTIME_NON_RX;\n    return result;\n  }}\n  if (ctx == NULL) return result;\n  if (runtime == NULL) goto release_event;\n\n  switch (event->message_id) {{\n"
    )
    .unwrap();
    for route in &profile.retained_routes {
        emit_retained_case(&mut output, module, &prefix, route);
    }
    write!(
        output,
        "    default:\n      result.domain = {prefix}_RUNTIME_UNKNOWN_MESSAGE;\n      break;\n  }}\n\nrelease_event:\n  wl_event_release(ctx, event);\n  return result;\n}}\n"
    )
    .unwrap();
    output
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
