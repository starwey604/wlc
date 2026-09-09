// SPDX-License-Identifier: Apache-2.0
use crate::profile_semantic::{BindingProfileModel, RpcService};
use std::fmt::Write;

pub(super) fn emit_pump_implementation(
    output: &mut String,
    profile: &BindingProfileModel,
    module: &str,
) {
    write!(
        output,
        "static wl_pump_event_disposition_t {module}_runtime_pump_event(void *user_data, wl_ctx_t *ctx, const wl_event_t *event, wl_time_ms_t now_ms) {{\n  {module}_runtime_pump_t *pump = ({module}_runtime_pump_t *)user_data;\n  {module}_runtime_result_t result;\n  if (pump == NULL || pump->runtime == NULL) return WL_PUMP_EVENT_UNHANDLED;\n  result = {module}_runtime_dispatch_event(ctx, event, pump->runtime, now_ms);\n  if (pump->on_result != NULL) pump->on_result(pump->user_data, &result);\n  return result.event_consumed != 0U ? WL_PUMP_EVENT_CONSUMED : WL_PUMP_EVENT_UNHANDLED;\n}}\n\n"
    )
    .unwrap();
    if profile.rpc_services.iter().any(RpcService::is_managed) {
        output.push_str(&crate::template::render(
            include_str!("../runtime_async_pump.c.in"),
            &[("M", module)],
        ));
    } else if !profile.rpc_services.is_empty() {
        write!(
            output,
            "static uint8_t {module}_runtime_pump_progress(void *user_data, wl_ctx_t *ctx, wl_time_ms_t now_ms) {{\n  {module}_runtime_pump_t *pump = ({module}_runtime_pump_t *)user_data;\n  if (pump == NULL || pump->runtime == NULL) return 0U;\n  pump->last_service_result = {module}_runtime_service(ctx, pump->runtime, now_ms, &pump->last_service);\n  if (pump->last_service_result != WL_RPC_OK) return 0U;\n  if (pump->last_service.response.message_id != 0U && pump->on_result != NULL)\n    pump->on_result(pump->user_data, &pump->last_service.response);\n  return pump->last_service.responses_submitted != 0U ? 1U : 0U;\n}}\n\nstatic uint32_t {module}_runtime_pump_deadline(const void *user_data, wl_time_ms_t now_ms) {{\n  const {module}_runtime_pump_t *pump = (const {module}_runtime_pump_t *)user_data;\n  wl_rpc_deadline_hint_t hint = {{0}};\n  if (pump == NULL || pump->runtime == NULL || {module}_runtime_get_deadline_hint(pump->runtime, now_ms, &hint) != WL_RPC_OK)\n    return WL_POLL_NO_DEADLINE_MS;\n  return hint.next_deadline_ms;\n}}\n\n"
        )
        .unwrap();
    }
    write!(
        output,
        "wl_err_t {module}_runtime_pump_init({module}_runtime_pump_t *pump, {module}_runtime_t *runtime, {module}_runtime_result_fn on_result, void *user_data) {{\n  if (pump == NULL || runtime == NULL) return WL_ERR_INVALID_ARG;\n  memset(pump, 0, sizeof(*pump));\n  pump->runtime = runtime;\n  pump->user_data = user_data;\n  pump->on_result = on_result;\n  return WL_OK;\n}}\n\nwl_pump_hooks_t {module}_runtime_pump_hooks({module}_runtime_pump_t *pump) {{\n  wl_pump_hooks_t hooks = {{0}};\n  if (pump == NULL) return hooks;\n  hooks.application_user_data = pump;\n  hooks.on_event = {module}_runtime_pump_event;\n"
    )
    .unwrap();
    if !profile.rpc_services.is_empty() {
        write!(
            output,
            "  hooks.application_progress = {module}_runtime_pump_progress;\n  hooks.application_deadline_hint = {module}_runtime_pump_deadline;\n"
        )
        .unwrap();
    }
    output.push_str("  return hooks;\n}\n");
}
