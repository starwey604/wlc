// SPDX-License-Identifier: Apache-2.0
use crate::codegen::{type_name, upper_snake};
use crate::profile_semantic::{DeliveryPolicy, RetainedRoute, RetainedRouteKind};
use std::fmt::Write;

pub(super) fn emit_retained_header_type(output: &mut String, module: &str, route: &RetainedRoute) {
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

pub(super) fn emit_retained_header_functions(
    output: &mut String,
    module: &str,
    route: &RetainedRoute,
) {
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

pub(super) fn emit_retained_implementation(
    output: &mut String,
    module: &str,
    route: &RetainedRoute,
) {
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

pub(super) fn emit_retained_case(
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
