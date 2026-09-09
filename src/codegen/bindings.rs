// SPDX-License-Identifier: Apache-2.0
use super::{type_name, upper_snake};
use crate::semantic::MessageSymbol;

pub(super) fn emit_bindings_header(
    module: &str,
    codec_guard: &str,
    messages: &[&MessageSymbol],
) -> String {
    let guard = format!("{codec_guard}_BINDINGS");
    let prefix = upper_snake(module);
    let mut output = format!(
        "#ifndef {guard}\n#define {guard}\n\n#include \"{module}.h\"\n#include <wirelink/link.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
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
        "typedef struct {{\n  uint8_t *data;\n  size_t capacity;\n}} {module}_encode_scratch_t;\n\ntypedef struct {{\n  {module}_send_domain_t domain;\n  wl_codec_status_t codec_status;\n  int core_result;\n  size_t payload_length;\n  wl_tx_handle_t handle;\n}} {module}_send_result_t;\n\n"
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
            "/* Encodes directly into Wirelink-owned TX storage. */\n{module}_send_result_t {module}_{name}_send(wl_ctx_t *ctx, const {name}_t *message, wl_delivery_t delivery, wl_time_ms_t now_ms);\n\n"
        ));
    }
    output.push_str("#ifdef __cplusplus\n}\n#endif\n\n#endif\n");
    output
}

pub(super) fn emit_bindings_source(module: &str, messages: &[&MessageSymbol]) -> String {
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
        output.push_str(&format!(
            "{module}_send_result_t {module}_{name}_send(wl_ctx_t *ctx, const {name}_t *message, wl_delivery_t delivery, wl_time_ms_t now_ms) {{\n  {module}_send_result_t result = {{ {prefix}_SEND_CORE_ERROR, WL_CODEC_OK, WL_OK, 0U, 0U }};\n  wl_tx_payload_claim_t claim = {{0}};\n  result.core_result = wl_tx_payload_claim(ctx, {macro_name}_MESSAGE_ID, delivery, &claim);\n  if (result.core_result != WL_OK) return result;\n  result.codec_status = {name}_encode(message, claim.span.data, claim.span.length, &result.payload_length);\n  if (result.codec_status != WL_CODEC_OK) {{\n    result.domain = {prefix}_SEND_CODEC_ERROR;\n    (void)wl_tx_payload_abort(ctx, &claim);\n    return result;\n  }}\n  result.core_result = wl_tx_payload_commit(ctx, &claim, result.payload_length, now_ms, delivery == WL_DELIVERY_RELIABLE ? &result.handle : NULL);\n  if (result.core_result != WL_OK) return result;\n  result.domain = {prefix}_SEND_OK;\n  return result;\n}}\n\n"
        ));
    }
    while output.ends_with("\n\n") {
        output.pop();
    }
    output
}
