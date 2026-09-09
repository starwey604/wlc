// SPDX-License-Identifier: Apache-2.0
use super::assembly::{emit_assembly_header, runtime_default_capacities};
use super::retained::{emit_retained_header_functions, emit_retained_header_type};
use super::rpc::{emit_rpc_header_functions, emit_rpc_header_types};
use crate::codegen::{c_identifier, type_name, upper_snake};
use crate::identity::{IDENTITY_ALGORITHM, binding_profile_identity, schema_identity};
use crate::manifest::CODEGEN_ABI_VERSION;
use crate::profile_semantic::{BindingProfileModel, RetainedRouteKind, RpcService};
use crate::semantic::SemanticModel;
use std::collections::HashMap;
use std::fmt::Write;

const RPC_FINGERPRINT_ALGORITHM: &str = "fnv1a64-canonical-request-v1";

pub(super) fn emit_header(
    schema: &SemanticModel,
    profile: &BindingProfileModel,
    codec_module: &str,
    module: &str,
    maxima: &HashMap<u16, Option<u64>>,
) -> String {
    let prefix = upper_snake(module);
    let guard = format!("WIRELINK_GENERATED_{prefix}_RUNTIME_H");
    let mut output = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include \"{codec_module}_bindings.h\"\n#include <wirelink/pump.h>\n#include <wirelink/endpoint.h>\n#include <wirelink/allocator.h>\n#include <wirelink/frame.h>\n#include <string.h>\n"
    );
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
    if profile.rpc_services.iter().any(RpcService::is_managed) {
        output.push_str("#include <wirelink/rpc_sync.h>\n");
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
            "typedef struct {{\n  wl_codec_status_t codec_status;\n  wl_rpc_err_t rpc_result;\n  int32_t core_result;\n  int32_t application_result;\n  wl_rpc_server_disposition_t rpc_disposition;\n  uint32_t operation_id;\n  wl_tx_handle_t handle;\n  uint8_t peer_changed;\n  size_t payload_length;\n  union {{\n    wl_rpc_server_request_t server_request;\n    wl_rpc_server_response_t server_response;\n  }};\n}} {module}_runtime_rpc_detail_t;\n\n"
        )
        .unwrap();
    }
    output.push_str("typedef union {\n");
    if profile.retained_routes.is_empty() && profile.rpc_services.is_empty() {
        output.push_str("  uint8_t _reserved;\n");
    }
    if !profile.retained_routes.is_empty() {
        writeln!(output, "  {module}_runtime_retained_detail_t retained;").unwrap();
    }
    if !profile.rpc_services.is_empty() {
        writeln!(output, "  {module}_runtime_rpc_detail_t rpc;").unwrap();
    }
    write!(
        output,
        "}} {module}_runtime_detail_t;\n\n/* Inspect detail only through the member selected by detail_kind. domain\n * classifies the outcome; zero-initialized unused detail fields retain their\n * corresponding success values. event_consumed is nonzero only when dispatch\n * released an RX event or reclaimed a terminal TX handle. */\ntypedef struct {{\n  {module}_runtime_domain_t domain;\n  wl_event_type_t event_type;\n  uint16_t message_id;\n  {module}_runtime_detail_kind_t detail_kind;\n  uint8_t event_consumed;\n  {module}_runtime_detail_t detail;\n}} {module}_runtime_result_t;\n\n"
    )
    .unwrap();
    emit_result_helpers_header(&mut output, profile, module, &prefix);
    writeln!(
        output,
        "#define {prefix}_RUNTIME_HAS_MANAGED_RPC {}",
        u8::from(profile.rpc_services.iter().any(RpcService::is_managed))
    )
    .unwrap();
    for route in &profile.retained_routes {
        emit_retained_header_type(&mut output, module, route);
    }
    for service in &profile.rpc_services {
        emit_rpc_header_types(&mut output, codec_module, module, service);
    }
    if !profile.rpc_services.is_empty() {
        if profile
            .rpc_services
            .iter()
            .any(|service| !service.is_managed())
        {
            output.push_str(
            "/* Shared by synchronous RPC encoders. Runtime APIs are owner-thread\n * operations and do not retain pointers to this scratch after return. */\ntypedef union {\n",
        );
            for service in profile
                .rpc_services
                .iter()
                .filter(|service| !service.is_managed())
            {
                let service_name = c_identifier(&service.name);
                let request = type_name(&service.request_name);
                let response = type_name(&service.response_name);
                writeln!(output, "  {request}_t {service_name}_request;").unwrap();
                writeln!(output, "  {response}_t {service_name}_response;").unwrap();
            }
            writeln!(output, "}} {module}_runtime_rpc_encode_scratch_t;\n").unwrap();
        }
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
        writeln!(
            output,
            "  wl_rpc_client_t *rpc_client;\n  wl_rpc_server_t *rpc_server;\n  wl_rpc_peer_t rpc_peer;\n  wl_rpc_peer_observation_t rpc_peer_observation;"
        )
        .unwrap();
        if profile.rpc_services.iter().any(RpcService::is_managed) {
            output.push_str("  uint64_t rpc_incarnation;\n  wl_rpc_async_t *rpc_async;\n");
        }
        if profile
            .rpc_services
            .iter()
            .any(|service| !service.is_managed())
        {
            writeln!(
                output,
                "  {module}_runtime_rpc_encode_scratch_t *rpc_encode_scratch;"
            )
            .unwrap();
        }
        for service in &profile.rpc_services {
            let service_name = c_identifier(&service.name);
            writeln!(output, "  {module}_{service_name}_rpc_t {service_name};").unwrap();
        }
    }
    writeln!(output, "}} {module}_runtime_t;\n").unwrap();
    write!(
        output,
        "typedef void (*{module}_runtime_result_fn)(void *user_data, const {module}_runtime_result_t *result);\n\ntypedef struct {{\n  {module}_runtime_t *runtime;\n  void *user_data;\n  {module}_runtime_result_fn on_result;\n"
    )
    .unwrap();
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  wl_rpc_err_t last_service_result;\n  {module}_runtime_service_result_t last_service;\n"
        )
        .unwrap();
    }
    writeln!(output, "}} {module}_runtime_pump_t;\n").unwrap();
    emit_assembly_header(&mut output, maxima, profile, module);
    output.push_str(concat!(
        "/* With non-null ctx/event every RX outcome is consumed. Matching RPC TX\n",
        " * terminal events advance the runtime and reclaim the handle. Inspect\n",
        " * result.event_consumed before applying a fallback owner action. */\n",
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
        writeln!(
            output,
            "/* Observe a nonzero point-to-point peer before non-RPC traffic is handled. */\nwl_rpc_err_t {module}_runtime_peer_observe(wl_ctx_t *ctx, {module}_runtime_t *runtime, uint64_t peer_session_id, wl_rpc_peer_observation_t *out_observation);"
        )
        .unwrap();
        write!(
            output,
            "/* A reliable server request automatically observes its peer session before\n * dispatch. Take a changed observation to revoke product leases/non-RPC work. */\nwl_rpc_err_t {module}_runtime_peer_observation_take({module}_runtime_t *runtime, wl_rpc_peer_observation_t *out_observation);\n/* Advance configured RPC deadlines without performing I/O. At most one\n * expired server identity is returned per call and remains pending until the\n * application completes, rejects, or abandons it. */\nwl_rpc_err_t {module}_runtime_poll({module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_poll_result_t *out_result);\n/* Advance deadlines and submit at most one runtime-owned server response.\n * Link backpressure defers the same cached bytes for a later service call. */\nwl_rpc_err_t {module}_runtime_service(wl_ctx_t *ctx, {module}_runtime_t *runtime, wl_time_ms_t now_ms, {module}_runtime_service_result_t *out_result);\n/* Side-effect free. Zero is due; WL_RPC_NO_DEADLINE_MS means no deadline. */\nwl_rpc_err_t {module}_runtime_get_deadline_hint(const {module}_runtime_t *runtime, wl_time_ms_t now_ms, wl_rpc_deadline_hint_t *out_hint);\n\n"
        )
        .unwrap();
    }
    write!(
        output,
        "/* Build pump hooks that dispatch events with the owner's time sample. RPC\n * profiles also service one queued response per pass and merge their deadline.\n * on_result may be null; result pointers are borrowed only for the callback. */\nwl_err_t {module}_runtime_pump_init({module}_runtime_pump_t *pump, {module}_runtime_t *runtime, {module}_runtime_result_fn on_result, void *user_data);\nwl_pump_hooks_t {module}_runtime_pump_hooks({module}_runtime_pump_t *pump);\n\n"
    )
    .unwrap();
    for service in &profile.rpc_services {
        emit_rpc_header_functions(&mut output, module, service);
    }
    output.push_str(&crate::endpoint_codegen::emit(
        maxima,
        profile,
        codec_module,
        module,
        runtime_default_capacities(maxima, profile).has_storage(),
    ));
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

pub(super) fn emit_result_helpers_header(
    output: &mut String,
    profile: &BindingProfileModel,
    module: &str,
    prefix: &str,
) {
    write!(
        output,
        "/* Convenience helpers preserve the full diagnostic result. Detail accessors\n * return null unless detail_kind selects the requested member. Result strings\n * are diagnostic text and must not be parsed as a stable machine interface. */\nstatic inline bool {module}_runtime_result_ok(const {module}_runtime_result_t *result) {{\n  return result != NULL && result->domain == {prefix}_RUNTIME_OK;\n}}\n\nconst char *{module}_runtime_result_str(const {module}_runtime_result_t *result);\n\n"
    )
    .unwrap();
    if !profile.retained_routes.is_empty() {
        write!(
            output,
            "static inline const {module}_runtime_retained_detail_t *{module}_runtime_result_retained_detail(const {module}_runtime_result_t *result) {{\n  return result != NULL && result->detail_kind == {prefix}_RUNTIME_DETAIL_RETAINED ? &result->detail.retained : NULL;\n}}\n\n"
        )
        .unwrap();
    }
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "static inline const {module}_runtime_rpc_detail_t *{module}_runtime_result_rpc_detail(const {module}_runtime_result_t *result) {{\n  return result != NULL && result->detail_kind == {prefix}_RUNTIME_DETAIL_RPC ? &result->detail.rpc : NULL;\n}}\n\n"
        )
        .unwrap();
    }
}
