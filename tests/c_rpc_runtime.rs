use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

const SCHEMA: &str = r#"
version 1;
enum RpcStatus = 1 { SUCCESS = 0; REJECTED = 7; }
message ComputeRequest = 2 {
  optional uint32 operation_id = 1;
  optional uint32 value = 2;
  optional bytes tag = 3;
}
message ComputeResponse = 3 {
  optional uint32 operation_id = 1;
  optional RpcStatus status = 2;
  optional uint32 output = 3;
  optional bytes proof = 4;
}
"#;

const PROFILE: &str = r#"
profile version 1;
rpc Compute {
  request = ComputeRequest;
  response = ComputeResponse;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = unreliable;
}
"#;

fn wirelink_root() -> std::path::PathBuf {
    fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("include"),
    )
    .unwrap()
    .parent()
    .unwrap()
    .to_path_buf()
}

#[test]
fn generated_rpc_runtime_executes_client_server_and_cache_lifecycles() {
    let directory = tempdir().unwrap();
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "rpc_fixture").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "rpc_fixture").unwrap();

    assert!(
        runtime
            .header
            .contains("RPC_FIXTURE_RPC_REQUEST_FINGERPRINT_ALGORITHM")
    );
    assert!(
        runtime
            .header
            .contains("rpc_fixture_compute_client_start_scratch")
    );
    assert!(
        runtime
            .header
            .contains("rpc_fixture_compute_client_start_direct")
    );
    assert!(
        runtime
            .header
            .contains("rpc_fixture_compute_server_retry_cached")
    );
    assert!(runtime.source.contains("wl_rpc_client_on_response"));
    assert!(runtime.source.contains("wl_rpc_server_begin"));
    assert!(
        runtime
            .source
            .contains("now_ms, &result.detail.rpc.rpc_disposition")
    );

    fs::write(directory.path().join("rpc_fixture.h"), codec.header).unwrap();
    fs::write(directory.path().join("rpc_fixture.c"), codec.source).unwrap();
    fs::write(
        directory.path().join("rpc_fixture_bindings.h"),
        codec.bindings_header,
    )
    .unwrap();
    fs::write(
        directory.path().join("rpc_fixture_bindings.c"),
        codec.bindings_source,
    )
    .unwrap();
    fs::write(
        directory.path().join("rpc_fixture_runtime.h"),
        runtime.header,
    )
    .unwrap();
    fs::write(
        directory.path().join("rpc_fixture_runtime.c"),
        runtime.source,
    )
    .unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "rpc_fixture_runtime.h"

#include <stdint.h>
#include <string.h>

static uint32_t release_calls;
static uint32_t zero_payload_on_release;
static uint32_t send_calls;
static uint32_t reliable_sends;
static uint16_t sent_message_id;
static uint8_t sent_payload[128];
static size_t sent_payload_length;
static int next_core_result = WL_OK;
static wl_tx_handle_t next_handle = 100U;
static uint8_t direct_payload[128];
static uint32_t direct_active;
static uint32_t handler_calls;
static uint32_t expected_release_in_handler;
static int32_t handler_result;

void wl_event_release(wl_ctx_t *ctx, const wl_event_t *event) {
  (void)ctx;
  ++release_calls;
  if (zero_payload_on_release != 0U && event->payload_len != 0U)
    memset((void *)(uintptr_t)event->payload, 0, event->payload_len);
}

int wl_send_unreliable(wl_ctx_t *ctx, uint16_t message_id,
                       const uint8_t *payload, size_t payload_len) {
  (void)ctx;
  ++send_calls;
  reliable_sends = 0U;
  sent_message_id = message_id;
  sent_payload_length = payload_len;
  if (payload_len != 0U) memcpy(sent_payload, payload, payload_len);
  return next_core_result;
}

int wl_send_reliable(wl_ctx_t *ctx, uint16_t message_id,
                     const uint8_t *payload, size_t payload_len,
                     wl_tx_handle_t *out_handle) {
  (void)ctx;
  ++send_calls;
  reliable_sends = 1U;
  sent_message_id = message_id;
  sent_payload_length = payload_len;
  if (payload_len != 0U) memcpy(sent_payload, payload, payload_len);
  if (next_core_result == WL_OK) *out_handle = next_handle++;
  return next_core_result;
}

int wl_tx_payload_claim(wl_ctx_t *ctx, uint16_t message_id,
                        wl_delivery_t delivery,
                        wl_tx_payload_claim_t *out_claim) {
  (void)ctx;
  (void)message_id;
  (void)delivery;
  if (direct_active != 0U) return WL_ERR_BUSY;
  direct_active = 1U;
  out_claim->span.data = direct_payload;
  out_claim->span.length = sizeof(direct_payload);
  out_claim->token = 44U;
  return WL_OK;
}

int wl_tx_payload_commit(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim,
                         size_t payload_len, wl_tx_handle_t *out_handle) {
  (void)ctx;
  if (direct_active == 0U || claim->token != 44U) return WL_ERR_NOT_FOUND;
  direct_active = 0U;
  ++send_calls;
  reliable_sends = out_handle != NULL ? 1U : 0U;
  sent_message_id = COMPUTE_REQUEST_MESSAGE_ID;
  sent_payload_length = payload_len;
  memcpy(sent_payload, claim->span.data, payload_len);
  if (out_handle != NULL) *out_handle = next_handle++;
  return next_core_result;
}

int wl_tx_payload_abort(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim) {
  (void)ctx;
  if (direct_active == 0U || claim->token != 44U) return WL_ERR_NOT_FOUND;
  direct_active = 0U;
  return WL_OK;
}

static int32_t handle_compute(void *user_data,
                              const compute_request_t *request,
                              wl_delivery_t delivery) {
  (void)user_data;
  ++handler_calls;
  if (release_calls != expected_release_in_handler) return -101;
  if (delivery != WL_DELIVERY_RELIABLE) return -102;
  if (!request->has_operation_id || request->operation_id == 0U ||
      !request->has_value || !request->has_tag || request->tag.length != 1U ||
      request->tag.data == NULL || request->tag.data[0] != 0xAAU)
    return -103;
  return handler_result;
}

static int init_runtime(rpc_fixture_runtime_t *runtime,
                        wl_rpc_client_t *client,
                        wl_rpc_server_t *server,
                        compute_request_t *request_scratch,
                        compute_response_t *response_scratch,
                        uint8_t *canonical, size_t canonical_size) {
  static wl_rpc_client_slot_t client_slots[2];
  static uint8_t client_responses[2][64];
  static wl_rpc_server_pending_slot_t pending[4];
  static wl_rpc_server_cache_slot_t cache[2];
  static uint8_t server_responses[2][64];
  const wl_rpc_client_config_t client_config = {
    .slots = client_slots,
    .slot_count = 2U,
    .response_storage = &client_responses[0][0],
    .response_storage_size = sizeof(client_responses),
    .response_capacity_per_slot = sizeof(client_responses[0]),
    .next_operation_id = 1U,
  };
  const wl_rpc_server_config_t server_config = {
    .pending_slots = pending,
    .pending_slot_count = 4U,
    .cache_slots = cache,
    .cache_slot_count = 2U,
    .response_storage = &server_responses[0][0],
    .response_storage_size = sizeof(server_responses),
    .response_capacity_per_slot = sizeof(server_responses[0]),
    .pending_timeout_ms = 5U,
    .cache_ttl_ms = 0U,
    .cache_policy = WL_RPC_CACHE_REJECT_NEW,
  };
  if (wl_rpc_client_init(client, &client_config) != WL_RPC_OK ||
      wl_rpc_server_init(server, &server_config) != WL_RPC_OK)
    return 1;
  memset(runtime, 0, sizeof(*runtime));
  runtime->rpc_client = client;
  runtime->rpc_server = server;
  runtime->compute.request_scratch = request_scratch;
  runtime->compute.response_scratch = response_scratch;
  runtime->compute.canonical_request_scratch =
      (rpc_fixture_encode_scratch_t){canonical, canonical_size};
  runtime->compute.request_handler = handle_compute;
  return 0;
}

static int check_client(wl_ctx_t *ctx, rpc_fixture_runtime_t *runtime,
                        wl_rpc_client_t *client) {
  uint8_t tag = 0x5AU;
  uint8_t proof = 0xC3U;
  uint8_t scratch[64];
  uint8_t response_bytes[64];
  uint8_t saved_response[64];
  size_t response_length = 0U;
  compute_request_t request = {0};
  compute_request_t decoded_request = {0};
  compute_response_t response = {0};
  compute_response_t inspected_response = {0};
  wl_event_t event = {0};
  wl_rpc_client_result_t client_result = {0};
  wl_rpc_deadline_hint_t deadline = {0};
  rpc_fixture_runtime_poll_result_t progress = {0};
  rpc_fixture_runtime_result_t result;

  request.has_value = true;
  request.value = 9U;
  request.has_tag = true;
  request.tag.data = &tag;
  request.tag.length = 1U;
  result = rpc_fixture_compute_client_start_scratch(
      ctx, runtime, &request, 100U, 10U,
      (rpc_fixture_encode_scratch_t){scratch, sizeof(scratch)});
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail_kind != RPC_FIXTURE_RUNTIME_DETAIL_RPC ||
      result.detail.rpc.operation_id != 1U ||
      !request.has_operation_id || request.operation_id != 1U ||
      reliable_sends != 1U || sent_message_id != COMPUTE_REQUEST_MESSAGE_ID)
    return 1;
  if (compute_request_decode(sent_payload, sent_payload_length,
                             &decoded_request) != WL_CODEC_OK ||
      decoded_request.operation_id != 1U || decoded_request.value != 9U)
    return 2;
  inspected_response.has_output = true;
  inspected_response.output = 999U;
  if (rpc_fixture_compute_client_inspect(runtime, 1U, &client_result) != WL_RPC_OK ||
      client_result.state != WL_RPC_CLIENT_LINK_PENDING)
    return 3;
  result = rpc_fixture_compute_client_decode(&client_result, &inspected_response);
  if (result.domain != RPC_FIXTURE_RUNTIME_RPC_ERROR ||
      result.detail.rpc.rpc_result != WL_RPC_ERR_INVALID_STATE ||
      inspected_response.has_output)
    return 4;

  event.type = WL_EVT_TX_SUCCESS;
  event.handle = result.detail.rpc.handle;
  result = rpc_fixture_runtime_dispatch_event(ctx, &event, runtime, 11U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK || release_calls != 0U) return 5;

  response.has_operation_id = true;
  response.operation_id = 1U;
  response.has_status = true;
  response.status = SUCCESS;
  response.has_output = true;
  response.output = 18U;
  response.has_proof = true;
  response.proof.data = &proof;
  response.proof.length = 1U;
  if (compute_response_encode(&response, response_bytes, sizeof(response_bytes),
                              &response_length) != WL_CODEC_OK)
    return 6;
  memcpy(saved_response, response_bytes, response_length);
  event.type = WL_EVT_UNRELIABLE_RX;
  event.message_id = COMPUTE_RESPONSE_MESSAGE_ID;
  event.payload = response_bytes;
  event.payload_len = response_length;
  zero_payload_on_release = 1U;
  result = rpc_fixture_runtime_dispatch_event(ctx, &event, runtime, 12U);
  zero_payload_on_release = 0U;
  if (result.domain != RPC_FIXTURE_RUNTIME_OK || release_calls != 1U ||
      result.detail.rpc.application_result != 0) return 7;
  memset(&inspected_response, 0, sizeof(inspected_response));
  if (rpc_fixture_compute_client_inspect(runtime, 1U, &client_result) != WL_RPC_OK ||
      client_result.state != WL_RPC_CLIENT_COMPLETED ||
      client_result.response_length != response_length ||
      memcmp(client_result.response_data, saved_response, response_length) != 0)
    return 8;
  result = rpc_fixture_compute_client_decode(&client_result, &inspected_response);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      !inspected_response.has_operation_id || inspected_response.operation_id != 1U ||
      !inspected_response.has_status || inspected_response.status != SUCCESS ||
      !inspected_response.has_output || inspected_response.output != 18U ||
      !inspected_response.has_proof || inspected_response.proof.length != 1U ||
      inspected_response.proof.data == NULL || inspected_response.proof.data[0] != 0xC3U)
    return 9;
  if (rpc_fixture_compute_client_release(runtime, 1U) != WL_RPC_OK ||
      rpc_fixture_compute_client_release(runtime, 1U) != WL_RPC_ERR_NOT_FOUND)
    return 10;

  memset(&request, 0, sizeof(request));
  request.has_value = true;
  request.value = 10U;
  result = rpc_fixture_compute_client_start_direct(
      ctx, runtime, &request, 100U, 20U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK || result.detail.rpc.operation_id != 2U ||
      request.operation_id != 2U || direct_active != 0U) return 11;
  if (wl_rpc_client_cancel(client, 2U) != WL_RPC_OK ||
      rpc_fixture_compute_client_release(runtime, 2U) != WL_RPC_OK) return 12;

  memset(&request, 0, sizeof(request));
  request.has_value = true;
  request.value = 11U;
  result = rpc_fixture_compute_client_start_scratch(
      ctx, runtime, &request, 100U, 30U,
      (rpc_fixture_encode_scratch_t){scratch, 1U});
  if (result.domain != RPC_FIXTURE_RUNTIME_CODEC_ERROR ||
      result.detail.rpc.operation_id != 3U || result.detail.rpc.codec_status != WL_CODEC_ERR_CAPACITY)
    return 13;
  if (rpc_fixture_compute_client_inspect(runtime, 3U, &client_result) != WL_RPC_OK ||
      client_result.state != WL_RPC_CLIENT_LINK_FAILED ||
      client_result.link_result != WL_ERR_CORRUPT_PAYLOAD ||
      rpc_fixture_compute_client_release(runtime, 3U) != WL_RPC_OK)
    return 14;

  memset(&request, 0, sizeof(request));
  request.has_value = true;
  request.value = 12U;
  result = rpc_fixture_compute_client_start_scratch(
      ctx, runtime, &request, 100U, 40U,
      (rpc_fixture_encode_scratch_t){scratch, sizeof(scratch)});
  if (result.domain != RPC_FIXTURE_RUNTIME_OK || result.detail.rpc.operation_id != 4U)
    return 15;
  memset(&response, 0, sizeof(response));
  response.has_operation_id = true;
  response.operation_id = 4U;
  if (compute_response_encode(&response, response_bytes, sizeof(response_bytes),
                              &response_length) != WL_CODEC_OK)
    return 16;
  event.type = WL_EVT_UNRELIABLE_RX;
  event.message_id = COMPUTE_RESPONSE_MESSAGE_ID;
  event.payload = response_bytes;
  event.payload_len = response_length;
  result = rpc_fixture_runtime_dispatch_event(ctx, &event, runtime, 41U);
  if (result.domain != RPC_FIXTURE_RUNTIME_RPC_ERROR ||
      result.detail.rpc.rpc_result != WL_RPC_ERR_RESPONSE_MISMATCH || release_calls != 2U)
    return 17;
  if (rpc_fixture_runtime_get_deadline_hint(runtime, 41U, &deadline) != WL_RPC_OK ||
      deadline.next_deadline_ms != 99U ||
      rpc_fixture_runtime_poll(runtime, 140U, &progress) != WL_RPC_OK ||
      progress.client_timed_out != 1U || progress.server_pending_expired != 0U ||
      progress.server_cache_expired != 0U)
    return 18;
  if (rpc_fixture_compute_client_inspect(runtime, 4U, &client_result) != WL_RPC_OK ||
      client_result.state != WL_RPC_CLIENT_TIMED_OUT ||
      rpc_fixture_compute_client_release(runtime, 4U) != WL_RPC_OK)
    return 19;

  event.type = WL_EVT_TX_FAILED;
  event.handle = UINT32_C(0xDEADBEEF);
  result = rpc_fixture_runtime_dispatch_event(ctx, &event, runtime, 31U);
  if (result.domain != RPC_FIXTURE_RUNTIME_NON_RX || release_calls != 2U)
    return 20;
  return 0;
}

static rpc_fixture_runtime_result_t dispatch_request(
    wl_ctx_t *ctx, rpc_fixture_runtime_t *runtime, uint8_t *payload,
    size_t payload_length, wl_time_ms_t now_ms) {
  wl_event_t event = {0};
  event.type = WL_EVT_RELIABLE_RX;
  event.message_id = COMPUTE_REQUEST_MESSAGE_ID;
  event.payload = payload;
  event.payload_len = payload_length;
  expected_release_in_handler = release_calls;
  return rpc_fixture_runtime_dispatch_event(ctx, &event, runtime, now_ms);
}

static int check_server(wl_ctx_t *ctx, rpc_fixture_runtime_t *runtime) {
  uint8_t request_a[] = {
    0x10U, 0x2AU, 0x08U, 0x4DU, 0x1AU, 0x01U, 0xAAU, 0x20U, 0x63U
  };
  uint8_t request_same[] = {
    0x08U, 0x4DU, 0x10U, 0x2AU, 0x1AU, 0x01U, 0xAAU
  };
  uint8_t request_conflict[] = {
    0x08U, 0x4DU, 0x10U, 0x2BU, 0x1AU, 0x01U, 0xAAU
  };
  uint8_t request_two[] = {
    0x08U, 0x4EU, 0x10U, 0x05U, 0x1AU, 0x01U, 0xAAU
  };
  uint8_t request_three[] = {
    0x08U, 0x4FU, 0x10U, 0x06U, 0x1AU, 0x01U, 0xAAU
  };
  uint8_t encoded[64];
  uint8_t first_cached[64];
  size_t first_cached_length;
  uint32_t sends_before;
  compute_response_t response = {0};
  compute_response_t decoded = {0};
  wl_rpc_server_response_t cached;
  wl_rpc_deadline_hint_t deadline = {0};
  rpc_fixture_runtime_poll_result_t progress = {0};
  rpc_fixture_runtime_result_t result;

  result = dispatch_request(ctx, runtime, request_a, sizeof(request_a), 100U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail_kind != RPC_FIXTURE_RUNTIME_DETAIL_RPC ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_NEW || handler_calls != 1U ||
      result.detail.rpc.operation_id != 77U) return 1;
  if (rpc_fixture_runtime_get_deadline_hint(runtime, 101U, &deadline) != WL_RPC_OK ||
      deadline.next_deadline_ms != 4U)
    return 2;
  result = dispatch_request(ctx, runtime, request_same, sizeof(request_same), 101U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_PENDING_DUPLICATE ||
      handler_calls != 1U) return 3;
  result = dispatch_request(ctx, runtime, request_conflict,
                            sizeof(request_conflict), 102U);
  if (result.domain != RPC_FIXTURE_RUNTIME_RPC_ERROR ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_CONFLICT ||
      result.detail.rpc.rpc_result != WL_RPC_ERR_OPERATION_CONFLICT || handler_calls != 1U)
    return 4;

  response.has_output = true;
  response.output = 84U;
  result = rpc_fixture_compute_server_complete(
      ctx, runtime, 77U, &response,
      (rpc_fixture_encode_scratch_t){encoded, sizeof(encoded)}, 103U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK || response.operation_id != 77U ||
      !response.has_status || response.status != SUCCESS ||
      reliable_sends != 0U || sent_message_id != COMPUTE_RESPONSE_MESSAGE_ID ||
      result.detail.rpc.server_response.response_data == NULL ||
      result.detail.rpc.payload_length != sent_payload_length ||
      memcmp(result.detail.rpc.server_response.response_data, sent_payload,
             sent_payload_length) != 0)
    return 5;
  first_cached_length = sent_payload_length;
  memcpy(first_cached, sent_payload, first_cached_length);
  cached = result.detail.rpc.server_response;
  result = rpc_fixture_compute_server_retry_cached(ctx, &cached);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      sent_payload_length != first_cached_length ||
      memcmp(sent_payload, first_cached, first_cached_length) != 0)
    return 6;

  result = dispatch_request(ctx, runtime, request_same, sizeof(request_same), 104U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_REPLAY || handler_calls != 1U ||
      sent_payload_length != first_cached_length ||
      memcmp(sent_payload, first_cached, first_cached_length) != 0)
    return 7;

  handler_result = -7;
  result = dispatch_request(ctx, runtime, request_two, sizeof(request_two), 105U);
  if (result.domain != RPC_FIXTURE_RUNTIME_APPLICATION_ERROR ||
      result.detail.rpc.application_result != -7 || handler_calls != 2U) return 8;
  handler_result = 0;
  result = dispatch_request(ctx, runtime, request_two, sizeof(request_two), 106U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_NEW || handler_calls != 3U)
    return 9;
  memset(&response, 0, sizeof(response));
  response.has_output = true;
  response.output = 10U;
  next_core_result = WL_ERR_BUSY;
  result = rpc_fixture_compute_server_reject(
      ctx, runtime, 78U, REJECTED, &response,
      (rpc_fixture_encode_scratch_t){encoded, sizeof(encoded)}, 107U);
  if (result.domain != RPC_FIXTURE_RUNTIME_CORE_ERROR ||
      result.detail.rpc.core_result != WL_ERR_BUSY ||
      result.detail.rpc.application_result != REJECTED || response.status != REJECTED ||
      compute_response_decode(sent_payload, sent_payload_length, &decoded) != WL_CODEC_OK ||
      decoded.operation_id != 78U || decoded.status != REJECTED)
    return 10;
  next_core_result = WL_OK;
  result = dispatch_request(ctx, runtime, request_two, sizeof(request_two), 108U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_REPLAY || handler_calls != 3U)
    return 11;

  result = dispatch_request(ctx, runtime, request_three, sizeof(request_three), 109U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_NEW || handler_calls != 4U)
    return 12;
  sends_before = send_calls;
  memset(&response, 0, sizeof(response));
  result = rpc_fixture_compute_server_reject(
      ctx, runtime, 79U, 0, &response,
      (rpc_fixture_encode_scratch_t){encoded, sizeof(encoded)}, 110U);
  if (result.domain != RPC_FIXTURE_RUNTIME_RPC_ERROR ||
      result.detail.rpc.rpc_result != WL_RPC_ERR_INVALID_ARG || send_calls != sends_before)
    return 13;
  result = rpc_fixture_compute_server_complete(
      ctx, runtime, 79U, &response,
      (rpc_fixture_encode_scratch_t){encoded, sizeof(encoded)}, 111U);
  if (result.domain != RPC_FIXTURE_RUNTIME_RPC_ERROR ||
      result.detail.rpc.rpc_result != WL_RPC_ERR_CACHE_FULL || send_calls != sends_before)
    return 14;
  if (rpc_fixture_runtime_poll(runtime, 115U, &progress) != WL_RPC_OK ||
      progress.client_timed_out != 0U ||
      progress.server_pending_expired != 1U ||
      progress.server_cache_expired != 0U)
    return 15;
  result = dispatch_request(ctx, runtime, request_three, sizeof(request_three), 116U);
  if (result.domain != RPC_FIXTURE_RUNTIME_OK ||
      result.detail.rpc.rpc_disposition != WL_RPC_SERVER_NEW || handler_calls != 5U)
    return 16;
  return 0;
}

int main(void) {
  wl_ctx_t ctx = {0};
  wl_rpc_client_t client = {0};
  wl_rpc_server_t server = {0};
  rpc_fixture_runtime_t runtime = {0};
  rpc_fixture_runtime_instance_t disabled_instance = {0};
  rpc_fixture_runtime_config_t disabled_config = {0};
  rpc_fixture_runtime_requirements_t disabled_requirements = {0};
  const rpc_fixture_runtime_storage_t disabled_storage = {NULL, 0U};
  compute_request_t request_scratch = {0};
  compute_response_t response_scratch = {0};
  uint8_t canonical[64];
  int result;
  if (rpc_fixture_runtime_requirements(&disabled_config,
                                       &disabled_requirements) != WL_OK ||
      disabled_requirements.storage_size != 0U ||
      disabled_requirements.storage_alignment != 1U ||
      rpc_fixture_runtime_init(&disabled_instance, &disabled_config,
                               &disabled_storage) != WL_OK ||
      disabled_instance.runtime.rpc_client != NULL ||
      disabled_instance.runtime.rpc_server != NULL)
    return 1;
  if (init_runtime(&runtime, &client, &server, &request_scratch,
                   &response_scratch, canonical, sizeof(canonical)) != 0)
    return 2;
  result = check_client(&ctx, &runtime, &client);
  if (result != 0) return 10 + result;
  result = check_server(&ctx, &runtime);
  return result == 0 ? 0 : 100 + result;
}
"#,
    )
    .unwrap();

    let root = wirelink_root();
    let executable = directory.path().join("rpc-runtime-test");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("rpc_fixture.c"))
        .arg(directory.path().join("rpc_fixture_bindings.c"))
        .arg(directory.path().join("rpc_fixture_runtime.c"))
        .arg(root.join("src/rpc.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "generated RPC runtime must compile cleanly"
    );
    let run = Command::new(&executable).status().unwrap();
    assert!(run.success(), "generated RPC runtime exited with {run}");

    fs::write(
        directory.path().join("rpc_header.cpp"),
        "#include \"rpc_fixture_runtime.h\"\nint main() { return 0; }\n",
    )
    .unwrap();
    let cxx = Command::new("c++")
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("rpc_header.cpp"))
        .status()
        .unwrap();
    assert!(cxx.success(), "generated RPC header must be C++20-clean");
}
