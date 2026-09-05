/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.h"
#include "peer_runtime.h"
#include "wirelink/loopback.h"
#include <stdio.h>

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)
static demo_endpoint_t a, b;
static wl_loopback_t cable;
static wl_time_ms_t now;
static demo_execute_request_token_t tokens[8];
static unsigned calls;
static int32_t inputs[8];
static wl_rpc_client_t concurrent_client;
static wl_rpc_client_slot_t client_slots[2];
static uint8_t client_responses[2][18];
static wl_rpc_server_t concurrent_server;
static wl_rpc_server_pending_slot_t pending_slots[2];
static wl_rpc_server_cache_slot_t cache_slots[4];
static uint8_t cached_responses[4][18];

static int32_t execute(void *user, const request_t *request,
                       const demo_execute_request_token_t *token,
                       wl_delivery_t delivery) {
  (void)user;
  if (calls >= 8U || delivery != (REQUEST_RELIABLE ? WL_DELIVERY_RELIABLE : WL_DELIVERY_UNRELIABLE)) return -1;
  tokens[calls] = *token;
  inputs[calls++] = request->input;
  return 0; /* Work is deferred; token and business input are copied. */
}

static int drain(void) {
  for (unsigned i = 0U; i < 8U; ++i, ++now) {
    CHECK(demo_endpoint_step(&a, now) == WL_OK);
    CHECK(demo_endpoint_step(&b, now) == WL_OK);
  }
  return 0;
}

static int init(void) {
  demo_endpoint_config_t ca, cb;
  const wl_rpc_client_config_t client = {
      client_slots, 2U, &client_responses[0][0], sizeof(client_responses), 18U, 1U};
  const wl_rpc_server_config_t server = {
      pending_slots, 2U, cache_slots, 4U, &cached_responses[0][0],
      sizeof(cached_responses), 18U, 1000U, 1000U, WL_RPC_CACHE_EVICT_OLDEST};
  CHECK(demo_endpoint_config_defaults(&ca, 101U) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&cb, 202U) == WL_OK);
  CHECK(demo_runtime_config_enable_client(&ca.runtime) == WL_OK);
  CHECK(demo_runtime_config_enable_server(&cb.runtime) == WL_OK);
  ca.link.ack_timeout_ms = cb.link.ack_timeout_ms = 10U;
  cb.runtime.execute_request_handler = execute;
  CHECK(demo_endpoint_init_config(&a, &ca) == WL_OK);
  CHECK(demo_endpoint_init_config(&b, &cb) == WL_OK);
  // Advanced test storage permits two concurrent calls, beyond the one-slot default.
  CHECK(wl_rpc_client_init(&concurrent_client, &client) == WL_RPC_OK);
  CHECK(wl_rpc_server_init(&concurrent_server, &server) == WL_RPC_OK);
  demo_endpoint_runtime(&a)->rpc_client = &concurrent_client;
  demo_endpoint_runtime(&b)->rpc_server = &concurrent_server;
  CHECK(wl_loopback_connect(&cable, demo_endpoint_handle(&a), demo_endpoint_handle(&b)) == WL_OK);
  calls = 0U;
  return 0;
}

static int call(int32_t value, uint32_t timeout, demo_execute_call_t *out) {
  request_t request;
  request_clear(&request);
  request.has_input = true;
  request.input = value;
  CHECK(demo_endpoint_execute_call(&a, &request, timeout, now, out) == WL_RPC_OK);
  CHECK(request.input == value && request.has_input);
  return drain();
}

static int complete(unsigned index, int32_t value) {
  response_t response;
  demo_runtime_result_t result;
  response_clear(&response);
  response.has_output = true;
  response.output = value;
  result = demo_endpoint_execute_complete(&b, &tokens[index], &response, now);
  CHECK(demo_runtime_result_ok(&result));
  return drain();
}

static void header(uint8_t *data, uint32_t id, int32_t status) {
  data[0] = 0U; data[1] = 1U; data[2] = 2U; data[3] = 0U;
  for (unsigned i = 0U; i < 4U; ++i) {
    data[4U + i] = (uint8_t)(id >> (24U - 8U * i));
    data[8U + i] = (uint8_t)((uint32_t)status >> (24U - 8U * i));
  }
}

static int inject(demo_endpoint_t *endpoint, uint16_t id, const uint8_t *data,
                   size_t length, bool reliable, wl_err_t expected) {
  static uint32_t sequence = 1000U;
  uint8_t wire[DEMO_ENDPOINT_UNIT_CAPACITY];
  size_t written;
  wl_wire_packet_t packet = {0};
  packet.type = WL_PACKET_DATA;
  packet.integrity = WL_INTEGRITY_CRC32C;
  packet.flags = reliable ? WL_PACKET_FLAG_RELIABLE : 0U;
  packet.sequence = reliable ? sequence++ : 0U;
  packet.session_id = endpoint == &a ? 202U : 101U;
  packet.message_id = id;
  packet.payload = data;
  packet.payload_len = length;
  CHECK(wl_frame_encode(&packet, WL_ENVELOPE_NATIVE_PACKET, wire, sizeof(wire), &written) == WL_OK);
  CHECK(wl_feed_unit(wl_endpoint_link(demo_endpoint_handle(endpoint)), wire, written) == WL_OK);
  CHECK(demo_endpoint_step(endpoint, now++) == expected);
  return 0;
}

static int inject_response(uint16_t id, const uint8_t *data, size_t length,
                           wl_err_t expected) {
  return inject(&a, id, data, length, RESPONSE_RELIABLE != 0, expected);
}

int main(void) {
  demo_execute_call_t first, second, saved;
  demo_execute_result_t result;
  demo_auxiliary_call_t wrong_service;
  auxiliary_request_t auxiliary;
  demo_auxiliary_result_t auxiliary_result;
  demo_runtime_result_t completed;
  demo_execute_request_token_t old_token;
  response_t response;
  request_t missing;
  uint8_t payload[18] = {0};
  uint32_t first_id, second_id;
  CHECK(init() == 0);
  CHECK(call(41, 1000U, &first) == 0);
  CHECK(call(50, 1000U, &second) == 0);
  CHECK(calls == 2U && inputs[0] == 41 && inputs[1] == 50);
  first_id = tokens[0].private_state.request.identity.operation_id;
  second_id = tokens[1].private_state.request.identity.operation_id;
  CHECK(first_id != second_id);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_ERR_INVALID_STATE);
  CHECK(demo_endpoint_execute_inspect(&b, &first, &result) != WL_RPC_OK);
  memcpy(&wrong_service, &first, sizeof(wrong_service));
  CHECK(demo_endpoint_auxiliary_inspect(&a, &wrong_service, &auxiliary_result) == WL_RPC_ERR_RESPONSE_MISMATCH);
  CHECK(complete(1U, 51) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &second, &result) == WL_RPC_OK);
  CHECK(result.state == WL_RPC_CLIENT_COMPLETED && result.response_valid && result.response.output == 51);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK);
  CHECK(result.state == WL_RPC_CLIENT_WAIT_RESPONSE && !result.response_valid);
  saved = second;
  CHECK(demo_endpoint_execute_release(&a, &second) == WL_RPC_OK);
  // A late released-call reply and malformed metadata must not complete first.
  header(payload, second_id, 0);
  payload[12] = 8U; payload[13] = 102U;
  CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 14U, WL_OK) == 0);
  CHECK(demo_endpoint_result(&a)->detail.rpc.rpc_result == WL_RPC_ERR_NOT_FOUND);
  header(payload, first_id, 0);
  for (unsigned byte = 0U; byte < 4U; ++byte) {
    payload[byte] ^= 0x80U;
    CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 14U, WL_ERR_INVALID_STATE) == 0);
    CHECK(demo_endpoint_result(&a)->detail.rpc.rpc_result == WL_RPC_ERR_MALFORMED_METADATA);
    payload[byte] ^= 0x80U;
  }
  CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 11U, WL_ERR_INVALID_STATE) == 0);
  header(payload, 0U, 0);
  CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 14U, WL_ERR_INVALID_STATE) == 0);
  CHECK(demo_endpoint_result(&a)->detail.rpc.rpc_result == WL_RPC_ERR_MALFORMED_METADATA);
  header(payload, first_id, 7);
  CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 14U, WL_ERR_INVALID_STATE) == 0);
  CHECK(demo_endpoint_result(&a)->detail.rpc.rpc_result == WL_RPC_ERR_MALFORMED_METADATA);
  header(payload, first_id, 0);
  CHECK(inject_response(RESPONSE_MESSAGE_ID, payload, 12U, WL_ERR_INVALID_STATE) == 0);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_CODEC_ERROR);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK);
  CHECK(result.state == WL_RPC_CLIENT_WAIT_RESPONSE);
  CHECK(complete(0U, 42) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK);
  CHECK(result.response_valid && result.response.output == 42);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_OK);
  header(payload, first_id, 0);
  payload[2] = 1U;
  payload[12] = 8U; payload[13] = 82U; /* Same request: input = 41. */
  CHECK(inject(&b, REQUEST_MESSAGE_ID, payload, 14U, REQUEST_RELIABLE != 0, WL_OK) == 0);
  CHECK(calls == 2U); /* Replay must not execute the handler again. */
  CHECK(drain() == 0);
  payload[13] = 84U;
  CHECK(inject(&b, REQUEST_MESSAGE_ID, payload, 14U, REQUEST_RELIABLE != 0, WL_ERR_INVALID_STATE) == 0);
  CHECK(demo_endpoint_result(&b)->detail.rpc.rpc_result == WL_RPC_ERR_OPERATION_CONFLICT);
  CHECK(calls == 2U);
  CHECK(drain() == 0);
  CHECK(call(-1, 1000U, &first) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &saved, &result) == WL_RPC_ERR_NOT_FOUND);
  completed = demo_endpoint_execute_reject(&b, &tokens[2], INT32_MIN, now);
  CHECK(demo_runtime_result_ok(&completed) && completed.detail.rpc.payload_length == 12U);
  CHECK(drain() == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK);
  CHECK(result.state == WL_RPC_CLIENT_APPLICATION_ERROR && result.application_status == INT32_MIN && !result.response_valid);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_OK);
  CHECK(call(9, 1000U, &first) == 0);
  CHECK(demo_endpoint_execute_cancel(&a, &first) == WL_RPC_OK);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK && result.state == WL_RPC_CLIENT_CANCELLED);
  CHECK(complete(3U, 10) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK && !result.response_valid);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_OK);
  CHECK(call(1, 1U, &first) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK && result.state == WL_RPC_CLIENT_TIMED_OUT);
  CHECK(complete(4U, 2) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &first, &result) == WL_RPC_OK && !result.response_valid);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_OK);
  request_clear(&missing);
  CHECK(demo_endpoint_execute_call(&a, &missing, 10U, now, &second) != WL_RPC_OK);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_CODEC_ERROR);
  CHECK(call(4, 1000U, &first) == 0); /* Failed encode did not leak a slot/claim. */
  saved = first;
  old_token = tokens[5];
  demo_endpoint_close(&a);
  demo_endpoint_close(&b);
  CHECK(init() == 0);
  CHECK(call(5, 1000U, &first) == 0);
  CHECK(demo_endpoint_execute_inspect(&a, &saved, &result) == WL_RPC_ERR_NOT_FOUND);
  response_clear(&response);
  response.has_output = true;
  response.output = 6;
  completed = demo_endpoint_execute_complete(&b, &old_token, &response, now);
  CHECK(!demo_runtime_result_ok(&completed));
  completed = demo_endpoint_execute_complete(&a, &tokens[0], &response, now);
  CHECK(!demo_runtime_result_ok(&completed));
  CHECK(complete(0U, 6) == 0);
  CHECK(demo_endpoint_execute_release(&a, &first) == WL_RPC_OK);
  // Empty business messages are valid and carry only managed metadata.
  auxiliary_request_clear(&auxiliary);
  CHECK(demo_endpoint_auxiliary_call(&a, &auxiliary, 10U, now, &wrong_service) == WL_RPC_OK);
  CHECK(demo_endpoint_auxiliary_cancel(&a, &wrong_service) == WL_RPC_OK);
  CHECK(demo_endpoint_auxiliary_release(&a, &wrong_service) == WL_RPC_OK);
  demo_endpoint_close(&a);
  demo_endpoint_close(&b);
  return 0;
}
