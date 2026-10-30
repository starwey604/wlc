/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.h"
#include "test_environment.h"
#include "wirelink/port.h"
#include <stdio.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)
#if MAPPED
#define TOKEN wl_rpc_server_request_t
#else
#define TOKEN demo_light_request_token_t
#endif
static TOKEN light_tokens[16];
static unsigned light_calls, strong_calls;
static wl_time_ms_t now_ms = 2U;
static int32_t light(void *user, const light_request_t *request, const TOKEN *token, wl_delivery_t delivery) {
  (void)user; (void)delivery;
  if (light_calls >= 16U || !request->has_value || request->value != 7U) return -1;
  light_tokens[light_calls++] = *token;
  return 0;
}
#if MAPPED
#define STRONG_TOKEN wl_rpc_server_request_t
#else
#define STRONG_TOKEN demo_strong_request_token_t
#endif
static int32_t strong(void *user, const strong_request_t *request, const STRONG_TOKEN *token, wl_delivery_t delivery) {
  (void)user; (void)token; (void)delivery;
  if (!request->has_value || request->value != 7U) return -1;
  ++strong_calls;
  return 0;
}
static demo_runtime_instance_t instance;
static union { wl_max_align_t align; uint8_t bytes[8192]; } arena;
static demo_endpoint_t endpoint;
#if LIGHT_CORRELATE && !REQUEST_RELIABLE
static const uint8_t *held_image;
static uint8_t held_copy[DEMO_ENDPOINT_UNIT_CAPACITY];
static size_t held_length;
static wl_io_token_t held_token;
static wl_sink_result_t hold_image(void *context, wl_io_token_t token,
                                   const uint8_t *data, size_t length) {
  (void)context;
  if (length > sizeof(held_copy)) return WL_SINK_FAILED;
  held_image = data; held_length = length; held_token = token;
  memcpy(held_copy, data, length);
  return WL_SINK_STARTED;
}
#endif

static wl_time_ms_t read_clock(void *unused) { (void)unused; return 1U; }
static demo_runtime_result_t dispatch(uint16_t id, uint64_t session, bool malformed) {
  uint8_t data[64] = {0};
  size_t n = 0U;
#if MAPPED
  data[n++] = 8U; data[n++] = 7U; /* required business value */
  data[n++] = 16U; data[n++] = 19U; /* mapped operation */
#else
  for (unsigned i = 0; i < 8U; ++i) data[n++] = (uint8_t)(session >> (56U - 8U * i));
  data[n++] = 38U; /* operation 19, no rejection */
#if POSITIONAL_STRONG
  data[n++] = id == STRONG_REQUEST_MESSAGE_ID ? 0U : 8U;
#else
  data[n++] = 8U;
#endif
  data[n++] = 7U;
#endif
  if (malformed) --n;
  wl_event_t event = {
      .type = REQUEST_RELIABLE ? WL_EVT_RELIABLE_RX : WL_EVT_UNRELIABLE_RX,
      .message_id = id, .payload = data, .payload_len = n,
      .peer_session_id = REQUEST_RELIABLE || MAPPED ? session : 0U,
  };
  return demo_runtime_dispatch_event(wl_endpoint_link(demo_endpoint_handle(&endpoint)), &event, &instance.runtime, now_ms);
}
int main(void) {
  demo_runtime_config_t config;
  demo_runtime_requirements_t required;
  CHECK(demo_endpoint_init(&endpoint, test_environment_id(101U, (wl_clock_t){read_clock, NULL}), wl_link_defaults()) == WL_OK);
  CHECK(demo_runtime_config_defaults(&config) == WL_OK);
  CHECK(demo_runtime_config_enable_server(&config) == WL_OK);
  config.rpc_server_pending_slot_count = 2U;
#if HAS_REPLAY
  config.rpc_server_cache_slot_count = 2U;
  config.rpc_server_cache_ttl_ms = 5U;
#endif
#if HAS_CORRELATE
  config.rpc_server_correlate_slot_count = 2U;
#endif
  config.rpc_server_pending_timeout_ms = 10U;
  config.light_request_handler = light;
  config.strong_request_handler = strong;
#if HAS_CORRELATE
  {
    demo_runtime_config_t invalid = config;
    invalid.rpc_server_correlate_slot_count = 0U;
    CHECK(demo_runtime_requirements(&invalid, &required) == WL_ERR_INVALID_ARG);
    invalid = config;
    invalid.rpc_server_correlate_response_capacity = 0U;
    CHECK(demo_runtime_requirements(&invalid, &required) == WL_ERR_INVALID_ARG);
#if HAS_REPLAY
    invalid = config;
    invalid.rpc_server_correlate_slot_count = UINT16_MAX;
    invalid.rpc_server_cache_slot_count = 1U;
    CHECK(demo_runtime_requirements(&invalid, &required) == WL_ERR_INVALID_ARG);
#endif
  }
#endif
  CHECK(demo_runtime_requirements(&config, &required) == WL_OK);
  CHECK(required.storage_size <= sizeof(arena.bytes));
  demo_runtime_storage_t storage = {.data = arena.bytes, .size = required.storage_size};
  CHECK(demo_runtime_init(&instance, &config, &storage) == WL_OK);
  demo_runtime_result_t result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result));
  CHECK(light_calls == 1U);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result));
#if LIGHT_CORRELATE
  CHECK(light_calls == 2U);
  /* Malformed body from a new peer cannot discard two active executions. */
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 202U, true);
  CHECK(result.domain == DEMO_RUNTIME_CODEC_ERROR);
  CHECK(instance.runtime.rpc_peer.session_id == 101U);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(result.domain == DEMO_RUNTIME_RPC_ERROR);
  CHECK(result.detail.rpc.rpc_result == WL_RPC_ERR_NO_SLOT);
  CHECK(light_calls == 2U);
#else
  CHECK(light_calls == 1U);
  CHECK(result.detail.rpc.rpc_disposition == WL_RPC_SERVER_PENDING_DUPLICATE);
#endif
  uint8_t bytes[200]; memset(bytes, 0x52, sizeof(bytes));
  light_response_t response; light_response_clear(&response);
  response.has_body = true; response.body = (wl_codec_bytes_t){bytes, sizeof(bytes)};
  result = demo_light_server_complete(&instance.runtime, &light_tokens[0], &response, 3U);
  CHECK(demo_runtime_result_ok(&result));
  now_ms = 4U;
  memset(bytes, 0x19, sizeof(bytes)); /* Encoded stable bytes already copied. */
  wl_rpc_server_response_t ready;
  CHECK(wl_rpc_server_response_acquire(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
  CHECK(ready.response_length >= 200U);
  light_response_t decoded;
  CHECK(light_response_decode(ready.response_data + (MAPPED ? 0U : 9U),
      ready.response_length - (MAPPED ? 0U : 9U), &decoded) == WL_CODEC_OK);
  CHECK(decoded.body.length == 200U);
  for (size_t i = 0U; i < decoded.body.length; ++i) CHECK(decoded.body.data[i] == 0x52U);
  CHECK(wl_rpc_server_response_defer(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
  CHECK(wl_rpc_server_response_acquire(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
#if LIGHT_CORRELATE
  CHECK(wl_rpc_server_response_submitted(instance.runtime.rpc_server, &ready, 55U) == WL_RPC_OK);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(result.domain == DEMO_RUNTIME_RPC_ERROR && light_calls == 2U); /* TX capacity held. */
  wl_event_t terminal = {.type = WL_EVT_TX_SUCCESS, .handle = 55U};
  CHECK(wl_rpc_server_on_tx_event(instance.runtime.rpc_server, &terminal) == WL_RPC_OK);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result) && light_calls == 3U);
  /* An old token cannot complete another execution of the same operation. */
  result = demo_light_server_complete(&instance.runtime, &light_tokens[0], &response, 4U);
  CHECK(result.domain == DEMO_RUNTIME_RPC_ERROR);
#else
  CHECK(wl_rpc_server_response_sent(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result) && light_calls == 1U);
  CHECK(result.detail.rpc.rpc_disposition == WL_RPC_SERVER_REPLAY);
  CHECK(wl_rpc_server_response_acquire(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
  CHECK(wl_rpc_server_response_sent(instance.runtime.rpc_server, &ready) == WL_RPC_OK);
#endif
  /* New valid peer retires old deferred generations only after validation. */
  result = dispatch(STRONG_REQUEST_MESSAGE_ID, 303U, false);
  CHECK(demo_runtime_result_ok(&result) && strong_calls == 1U);
  CHECK(instance.runtime.rpc_peer.session_id == 303U);
  result = demo_light_server_complete(&instance.runtime, &light_tokens[LIGHT_CORRELATE ? 1 : 0], &response, 5U);
  CHECK(result.domain == DEMO_RUNTIME_RPC_ERROR);
  result = dispatch(STRONG_REQUEST_MESSAGE_ID, 303U, false);
  CHECK(demo_runtime_result_ok(&result));
  CHECK(strong_calls == (STRONG_CORRELATE ? 2U : 1U));
  demo_runtime_poll_result_t poll;
  CHECK(demo_runtime_poll(&instance.runtime, 14U, &poll) == WL_RPC_OK);
  CHECK(poll.server_pending_expired == 1U);
#if LIGHT_CORRELATE && !REQUEST_RELIABLE
  /* A released transient response may be reused while the adapter still owns
   * the first link TX image. Backpressure must preserve both independent lives. */
  config.rpc_server_pending_slot_count = 1U;
  config.rpc_server_correlate_slot_count = 1U;
  CHECK(demo_runtime_init(&instance, &config, &storage) == WL_OK);
  light_calls = 0U; now_ms = 20U;
  wl_ctx_t *link = wl_endpoint_link(demo_endpoint_handle(&endpoint));
  CHECK(wl_set_sink(link, hold_image, NULL) == WL_OK);
  CHECK(wl_set_tx_drain_required(link, 1U) == WL_OK);
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result) && light_calls == 1U);
  memset(bytes, 0x52, sizeof(bytes));
  result = demo_light_server_complete(&instance.runtime, &light_tokens[0], &response, 21U);
  CHECK(demo_runtime_result_ok(&result));
  demo_runtime_service_result_t service;
  CHECK(demo_runtime_service(link, &instance.runtime, 21U, &service) == WL_RPC_OK);
  CHECK(service.responses_submitted == 1U && held_length != 0U);
  now_ms = 22U;
  result = dispatch(LIGHT_REQUEST_MESSAGE_ID, 101U, false);
  CHECK(demo_runtime_result_ok(&result) && light_calls == 2U);
  memset(bytes, 0x19, sizeof(bytes));
  result = demo_light_server_complete(&instance.runtime, &light_tokens[1], &response, 23U);
  CHECK(demo_runtime_result_ok(&result));
  CHECK(memcmp(held_image, held_copy, held_length) == 0);
  CHECK(demo_runtime_service(link, &instance.runtime, 23U, &service) == WL_RPC_OK);
  CHECK(service.responses_submitted == 0U); /* Physical image is still borrowed. */
  CHECK(memcmp(held_image, held_copy, held_length) == 0);
  CHECK(wl_tx_consumed(link, held_token, WL_OK) == WL_OK);
  CHECK(demo_runtime_service(link, &instance.runtime, 23U, &service) == WL_RPC_OK);
  CHECK(service.responses_submitted == 0U); /* Physical idle is a distinct boundary. */
  CHECK(wl_tx_drained(link, held_token, WL_OK) == WL_OK);
  CHECK(demo_runtime_service(link, &instance.runtime, 23U, &service) == WL_RPC_OK);
  CHECK(service.responses_submitted == 1U);
  CHECK(wl_tx_consumed(link, held_token, WL_OK) == WL_OK);
  CHECK(wl_tx_drained(link, held_token, WL_OK) == WL_OK);
#endif
  CHECK(demo_endpoint_close(&endpoint) == WL_OK);
  return 0;
}
