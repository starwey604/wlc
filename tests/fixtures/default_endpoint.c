/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.h"
#include "peer_runtime.h"
#include <stdio.h>
#include "wirelink/loopback.h"

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)

static demo_endpoint_t a, b;
static wl_time_ms_t server_time;
static unsigned calls;

static int32_t execute(void *context, const request_t *request,
                       const wl_rpc_server_request_t *token, wl_delivery_t delivery) {
  response_t response;
  demo_runtime_result_t result;
  (void)delivery;
  response_clear(&response);
  response.has_output = true;
  response.output = request->input + 1;
  ++calls;
  result = demo_endpoint_execute_complete(context, token, &response, server_time);
  return demo_runtime_result_ok(&result) ? 0 : -1;
}

static int exercise(void) {
  demo_endpoint_config_t client_config, server_config;
  wl_loopback_t cable, other;
  state_t state, read;
  alarm_t alarm, read_alarm;
  request_t request;
  response_t response;
  demo_runtime_result_t result;
  const demo_runtime_rpc_detail_t *detail;
  wl_rpc_client_result_t operation;
  wl_poll_hint_t hint;
  uint32_t id;

  CHECK(demo_endpoint_step(&a, 0) == WL_ERR_NOT_INITIALIZED);
  CHECK(demo_endpoint_init(&a, 0) == WL_ERR_INVALID_ARG);
  CHECK(demo_endpoint_config_defaults(&client_config, 1) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&server_config, 2) == WL_OK);
  CHECK(demo_runtime_config_enable_client(&client_config.runtime) == WL_OK);
  CHECK(demo_runtime_config_enable_server(&server_config.runtime) == WL_OK);
  client_config.link.ack_timeout_ms = 5;
  server_config.link.ack_timeout_ms = 5;
  server_config.runtime.execute_request_handler = execute;
  server_config.runtime.execute_user_data = &b;
  server_config.runtime.rpc_server_pending_timeout_ms = 100;
  server_config.runtime.rpc_server_cache_ttl_ms = 1000;
  CHECK(demo_endpoint_init_config(&a, &client_config) == WL_OK);
  CHECK(demo_endpoint_init_config(&b, &server_config) == WL_OK);
  CHECK(demo_endpoint_init(&a, 7) == WL_ERR_INVALID_STATE);
  CHECK(wl_loopback_connect(&cable, demo_endpoint_handle(&a), demo_endpoint_handle(&b)) == WL_OK);
  CHECK(wl_loopback_connect(&other, demo_endpoint_handle(&a), demo_endpoint_handle(&b)) == WL_ERR_BUSY);

  state_clear(&state);
  state.has_sequence = true;
  for (uint32_t i = 1; i <= 2; ++i) {
    state.sequence = i;
    CHECK(demo_endpoint_send_state(&a, &state).domain == DEMO_SEND_OK);
    CHECK(wl_endpoint_get_hint(demo_endpoint_handle(&a), i, &hint) == WL_OK);
    CHECK(hint.next_deadline_ms == 0);
    CHECK(demo_endpoint_step(&a, i) == WL_OK);
    CHECK(demo_endpoint_step(&b, i) == WL_OK);
  }
  CHECK(demo_endpoint_read_state(&b, &read) == WL_OK && read.sequence == 2);
  CHECK(demo_endpoint_read_state(&b, &read) == WL_ERR_NO_DATA && read.sequence == 2);
  CHECK(demo_endpoint_read_state(&b, NULL) == WL_ERR_INVALID_ARG);

  alarm_clear(&alarm);
  alarm.has_code = true;
  alarm.code = 17;
  CHECK(demo_endpoint_send_alarm(&a, &alarm).domain == DEMO_SEND_OK);
  CHECK(demo_endpoint_step(&a, 3) == WL_OK);
  CHECK(demo_endpoint_step(&b, 3) == WL_OK);
  CHECK(demo_endpoint_step(&a, 4) == WL_OK); /* Non-RPC reliable completion. */
  CHECK(demo_endpoint_read_alarm(&b, &read_alarm) == WL_OK && read_alarm.code == 17);
  CHECK(demo_endpoint_read_alarm(&b, &read_alarm) == WL_ERR_NO_DATA);

  request_clear(&request);
  request.has_input = true;
  request.input = 41;
  result = demo_endpoint_execute_start(&a, &request, 100, 10);
  detail = demo_runtime_result_rpc_detail(&result);
  CHECK(demo_runtime_result_ok(&result) && detail != NULL);
  id = detail->operation_id;
  for (server_time = 10; server_time < 30; ++server_time) {
    CHECK(demo_endpoint_step(&a, server_time) == WL_OK);
    CHECK(demo_endpoint_step(&b, server_time) == WL_OK);
  }
  CHECK(calls == 1);
  CHECK(demo_endpoint_execute_inspect(&a, id, &operation) == WL_RPC_OK);
  CHECK(operation.state == WL_RPC_CLIENT_COMPLETED);
  result = demo_execute_client_decode(&operation, &response);
  CHECK(demo_runtime_result_ok(&result));
  CHECK(response.output == 42);
  CHECK(demo_endpoint_execute_release(&a, id) == WL_RPC_OK);
  demo_endpoint_close(&a);
  demo_endpoint_close(&b);
  demo_endpoint_close(&a);
  CHECK(demo_endpoint_step(&a, 30) == WL_ERR_NOT_INITIALIZED);
  return 0;
}

static wl_sink_result_t blackhole(void *context, wl_io_token_t token,
                                  const uint8_t *bytes, size_t length) {
  (void)context; (void)token; (void)bytes; (void)length;
  return WL_SINK_SENT;
}

static int failures(void) {
  demo_endpoint_config_t config;
  uint8_t wire[DEMO_ENDPOINT_UNIT_CAPACITY], payload[DEMO_ENDPOINT_MAX_PAYLOAD];
  size_t first, second, length, accepted;
  wl_wire_packet_t packet = {0};
  state_t value, read;
  alarm_t alarm;
  CHECK(demo_endpoint_config_defaults(&config, 7) == WL_OK);
  config.link.envelope = WL_ENVELOPE_COBS_STREAM;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
  packet.type = WL_PACKET_DATA;
  packet.integrity = WL_INTEGRITY_CRC32C;
  packet.message_id = 999; /* Follow an unknown route with a valid message. */
  CHECK(wl_frame_encode(&packet, WL_ENVELOPE_COBS_STREAM, wire, sizeof(wire), &first) == WL_OK);
  state_clear(&value);
  value.has_sequence = true;
  value.sequence = 8;
  CHECK(state_encode(&value, payload, sizeof(payload), &length) == WL_CODEC_OK);
  packet.message_id = STATE_MESSAGE_ID;
  packet.payload = payload;
  packet.payload_len = length;
  CHECK(wl_frame_encode(&packet, WL_ENVELOPE_COBS_STREAM, wire + first, sizeof(wire) - first, &second) == WL_OK);
  CHECK(wl_feed_bytes(wl_endpoint_link(demo_endpoint_handle(&a)), wire, first + second, &accepted) == WL_OK);
  CHECK(accepted == first + second);
  CHECK(demo_endpoint_step(&a, 1) == WL_ERR_INVALID_STATE);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_UNKNOWN_MESSAGE);
  CHECK(demo_endpoint_read_state(&a, &read) == WL_OK && read.sequence == 8);
  CHECK(demo_endpoint_step(&a, 2) == WL_OK); /* Previous error does not poison a new pass. */
  demo_endpoint_close(&a);

  CHECK(demo_endpoint_config_defaults(&config, 8) == WL_OK);
  config.link.ack_timeout_ms = 5;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
  CHECK(wl_set_sink(wl_endpoint_link(demo_endpoint_handle(&a)), blackhole, NULL) == WL_OK);
  alarm_clear(&alarm);
  alarm.has_code = true;
  alarm.code = 3;
  CHECK(demo_endpoint_send_alarm(&a, &alarm).domain == DEMO_SEND_OK);
  CHECK(demo_endpoint_step(&a, 1) == WL_OK);
  CHECK(demo_endpoint_step(&a, 10) == WL_ERR_INVALID_STATE);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_CORE_ERROR);
  demo_endpoint_close(&a);
  return 0;
}

static int configurations(void) {
  demo_endpoint_config_t config;
  wl_storage_requirements_t requirements;
  for (int envelope = WL_ENVELOPE_COBS_STREAM; envelope <= WL_ENVELOPE_BUS_LENGTH16; ++envelope) {
    for (int integrity = WL_INTEGRITY_NONE; integrity <= WL_INTEGRITY_CRC32C; ++integrity) {
      CHECK(demo_endpoint_config_defaults(&config, 9) == WL_OK);
      config.link.envelope = envelope;
      config.link.integrity = integrity;
      CHECK(wl_config_requirements(&config.link, &requirements) == WL_OK);
      CHECK(requirements.tx_unit_size <= DEMO_ENDPOINT_UNIT_CAPACITY);
      CHECK(requirements.control_unit_size <= DEMO_ENDPOINT_CONTROL_CAPACITY);
      CHECK(requirements.rx_fifo_size <= DEMO_ENDPOINT_UNIT_CAPACITY);
      CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
      demo_endpoint_close(&a);
    }
  }
  CHECK(demo_endpoint_config_defaults(&config, 9) == WL_OK);
  config.link.max_payload_len = DEMO_ENDPOINT_MAX_PAYLOAD + 1;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_BUF_TOO_SMALL);
  CHECK(wl_endpoint_link(demo_endpoint_handle(&a)) == NULL);
  config.link.max_payload_len = DEMO_ENDPOINT_MAX_PAYLOAD;
  config.event_budget = 0;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_INVALID_ARG);
  return 0;
}

int main(void) {
  CHECK(exercise() == 0);
  CHECK(failures() == 0);
  CHECK(configurations() == 0);
  return 0;
}
