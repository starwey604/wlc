/* SPDX-License-Identifier: Apache-2.0 */
#include "wire_runtime.c"
#include <assert.h>

static void headers(void) {
  const uint32_t ids[] = {1U, 63U, 64U, 127U, 128U, 8191U, 8192U, UINT32_C(1) << 27, UINT32_MAX};
  const uint64_t sessions[] = {1U, UINT64_C(0x0123456789ABCDEF), UINT64_MAX};
  const int32_t statuses[] = {0, 1, -1, INT32_MIN, INT32_MAX};
  uint8_t bytes[18];
  for (size_t i = 0; i < sizeof(ids) / sizeof(ids[0]); ++i) {
    for (size_t j = 0; j < sizeof(sessions) / sizeof(sessions[0]); ++j) {
      for (size_t k = 0; k < sizeof(statuses) / sizeof(statuses[0]); ++k) {
        uint32_t id; uint64_t session; int32_t status; size_t read;
        size_t length = wire_rpc_header_write(bytes, 2U, ids[i], statuses[k], sessions[j]);
        assert(length == wire_rpc_header_size(ids[i], statuses[k]));
        assert(length <= (statuses[k] ? 17U : 13U));
        assert(wire_rpc_header_read(bytes, length, 2U, &id, &status, &session, &read) == WL_RPC_OK);
        assert(id == ids[i] && session == sessions[j] && status == statuses[k] && read == length);
        assert(wire_rpc_header_read(bytes, length, 1U, &id, &status, &session, &read) == (statuses[k] ? WL_RPC_ERR_MALFORMED_METADATA : WL_RPC_OK));
        for (size_t truncated = 0; truncated < length; ++truncated)
          assert(wire_rpc_header_read(bytes, truncated, 2U, &id, &status, &session, &read) == WL_RPC_ERR_MALFORMED_METADATA);
      }
    }
  }
}
static void payloads(void) {
  const uint32_t values[] = {0U, 1U, 63U, 64U, 127U, 128U, 16383U, 16384U, UINT32_MAX};
  request_t request = {.has_first = true, .has_second = true, .has_third = true}, decoded;
  uint8_t encoded[32], public_wire[32]; size_t length, public_length;
  for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
    request.first = values[i]; request.second = ~values[i]; request.third = values[i] ^ UINT32_C(0xA5A5A5A5);
    assert(wire_rpc_request_encode(&request, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
    assert(encoded[0] == 8U); // Tagged varuint32 inside managed RPC.
    assert(wire_rpc_request_decode(encoded, length, &decoded) == WL_CODEC_OK);
    assert(decoded.has_first && decoded.has_second && decoded.has_third && decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
    assert(request_encode(&request, public_wire, sizeof(public_wire), &public_length) == WL_CODEC_OK);
    assert(public_wire[0] == 13U && public_length == 15U); // Frozen public Schema-v1 stays fixed32.
    encoded[length] = 0x20U; encoded[length + 1U] = 0x96U; encoded[length + 2U] = 0x01U; // A newly added integer field.
    assert(wire_rpc_request_decode(encoded, length + 3U, &decoded) == WL_CODEC_OK);
    assert(decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
  }
  const uint8_t malformed[] = {8U, 0x80U, 0U, 16U, 1U, 24U, 2U};
  assert(wire_rpc_request_decode(malformed, sizeof(malformed), &decoded) == WL_CODEC_ERR_MALFORMED);
  const uint8_t overflow[] = {8U, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0x1FU, 16U, 1U, 24U, 2U};
  assert(wire_rpc_request_decode(overflow, sizeof(overflow), &decoded) == WL_CODEC_ERR_OVERFLOW);
  // Nested fixed32 changes consistently, while packed fixed32 stays fixed width.
  response_t response = {.has_reply = true, .reply = UINT32_MAX, .has_nested = true}, got;
  response.nested.has_scalar = response.nested.has_array = true;
  response.nested.scalar = UINT32_MAX;
  response.nested.array[0] = 0U; response.nested.array[1] = UINT32_MAX;
  assert(wire_rpc_response_encode(&response, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_response_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_nested && got.nested.has_scalar && got.nested.scalar == UINT32_MAX && got.nested.has_array && got.nested.array[1] == UINT32_MAX);
}
static void names(void) {
  uint8_t encoded[16]; size_t length;
  wlc_foo_t one = {.has_wlc_foo_encode = true, .wlc_foo_encode = UINT32_MAX}, decoded;
  assert(wire_rpc_wlc_foo_encode(&one, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_wlc_foo_decode(encoded, length, &decoded) == WL_CODEC_OK);
  assert(decoded.has_wlc_foo_encode && decoded.wlc_foo_encode == UINT32_MAX);
  request_extra_t two = {.has_request_extra_encode = true, .request_extra_encode = UINT32_MAX,
      .has_wlc_clear = true, .wlc_clear = 19U}, got;
  assert(wire_rpc_request_extra_encode(&two, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_request_extra_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_request_extra_encode && got.request_extra_encode == UINT32_MAX && got.has_wlc_clear && got.wlc_clear == 19U);
}
static wl_time_ms_t now(void *context) { (void)context; return 10U; }
static wl_err_t session(void *context, uint64_t *out) { (void)context; *out = UINT64_MAX; return WL_OK; }
static void completed_owned(void) {
  static wire_endpoint_t endpoint;
  static wl_rpc_client_t client;
  static wl_rpc_client_slot_t slot;
  wire_endpoint_config_t config;
  const wl_environment_t environment = {{now, NULL}, {session, NULL}};
  assert(wire_endpoint_config_defaults(&config, environment, wl_link_defaults()) == WL_OK);
  assert(wire_endpoint_init_config(&endpoint, &config) == WL_OK);
  wire_runtime_t *runtime = wire_endpoint_runtime(&endpoint);
  wl_ctx_t *link = wl_endpoint_link(wire_endpoint_handle(&endpoint));
  runtime->rpc_client = &client;
  const uint32_t ids[] = {1U, 63U, 64U, 8191U, 8192U, UINT32_C(1) << 27, UINT32_MAX};
  response_t response = {.has_reply = true, .reply = UINT32_MAX};
  for (size_t i = 0; i < sizeof(ids) / sizeof(ids[0]); ++i) {
    uint8_t payload[64]; size_t body_length;
    uint32_t id;
    const wl_rpc_client_config_t storage = {&slot, 1U, ids[i]};
    assert(wl_rpc_client_init(&client, &storage) == WL_RPC_OK);
    assert(wl_rpc_client_begin(&client, REQUEST_MESSAGE_ID, RESPONSE_MESSAGE_ID, 1000U, 10U, &id) == WL_RPC_OK && id == ids[i]);
    assert(wl_rpc_client_tx_completed(&client, id) == WL_RPC_OK);
    size_t head = wire_rpc_header_write(payload, 2U, id, 0, UINT64_MAX);
    assert(wire_rpc_response_encode(&response, payload + head, sizeof(payload) - head, &body_length) == WL_CODEC_OK);
    wl_event_t event = {.type = WL_EVT_UNRELIABLE_RX, .message_id = RESPONSE_MESSAGE_ID,
        .payload = payload, .payload_len = head + body_length};
    wl_rpc_client_result_t raw;
    const unsigned before = response_decode_count;
    // Stale or malformed responses cannot reach the completed owned preparation.
    payload[7] ^= 1U;
    wire_runtime_result_t result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.detail.rpc.rpc_result == WL_RPC_ERR_SESSION_MISMATCH);
    payload[7] ^= 1U;
    const uint8_t opcode = payload[8]; payload[8] = 0U;
    result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.detail.rpc.rpc_result == WL_RPC_ERR_MALFORMED_METADATA);
    payload[8] = opcode;
    assert(response_decode_count == before);
    const uint8_t tag = payload[head]; payload[head] = 0U;
    result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.domain == WIRE_RUNTIME_CODEC_ERROR);
    payload[head] = tag;
    assert(response_decode_count == before + 1U);
    assert(wl_rpc_client_get(&client, id, &raw) == WL_RPC_OK && raw.state == WL_RPC_CLIENT_WAIT_RESPONSE);
    result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.domain == WIRE_RUNTIME_OK);
    assert(wl_rpc_client_get(&client, id, &raw) == WL_RPC_OK && raw.state == WL_RPC_CLIENT_COMPLETED);
    assert(response_decode_count == before + 2U);
    const response_value_t *borrowed;
    assert(wire_execute_client_response(runtime, id, &borrowed) == WL_RPC_OK);
    response_value_t saved = *borrowed;
    /* Duplicate with changed body cannot overwrite the completed member. */
    payload[head + 1U] ^= 1U;
    result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.detail.rpc.rpc_result == WL_RPC_ERR_INVALID_STATE);
    assert(response_decode_count == before + 2U && borrowed->reply == UINT32_MAX);
    wire_endpoint_execute_prepare(&endpoint, &raw);
    assert(response_decode_count == before + 2U);
    assert(endpoint.private_state.completion.status == WL_RPC_SUCCESS);
    assert(endpoint.private_state.completion_value.execute.has_reply && endpoint.private_state.completion_value.execute.reply == UINT32_MAX);
    assert(wl_rpc_client_release(&client, id) == WL_RPC_OK);
    memset(runtime->rpc_results, 0, sizeof(*runtime->rpc_results));
    assert(saved.reply == UINT32_MAX);
    // A separate fresh client reservation covers rejection at the same ID width.
    // UINT32_MAX correctly exhausts the preceding client incarnation.
    assert(wl_rpc_client_init(&client, &storage) == WL_RPC_OK);
    // Rejection never invokes the successful owned body decoder.
    assert(wl_rpc_client_begin(&client, REQUEST_MESSAGE_ID, RESPONSE_MESSAGE_ID, 1000U, 10U, &id) == WL_RPC_OK);
    assert(wl_rpc_client_tx_completed(&client, id) == WL_RPC_OK);
    event.payload_len = wire_rpc_header_write(payload, 2U, id, INT32_MIN, UINT64_MAX);
    result = wire_runtime_dispatch_event(link, &event, runtime, 10U);
    assert(result.domain == WIRE_RUNTIME_OK);
    assert(wl_rpc_client_get(&client, id, &raw) == WL_RPC_OK && raw.state == WL_RPC_CLIENT_APPLICATION_ERROR);
    wire_endpoint_execute_prepare(&endpoint, &raw);
    assert(endpoint.private_state.completion.status == WL_RPC_REJECTED && endpoint.private_state.completion.rejection == INT32_MIN);
    assert(response_decode_count == before + 2U);
    assert(wl_rpc_client_release(&client, id) == WL_RPC_OK);
  }
  assert(wire_endpoint_close(&endpoint) == WL_OK);
}
int main(void) { headers(); payloads(); names(); completed_owned(); return 0; }
