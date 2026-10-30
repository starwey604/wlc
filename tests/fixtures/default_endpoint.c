/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.h"
#include "test_environment.h"
#include "peer_runtime.h"
#include <stdio.h>
#include "wirelink/loopback.h"
#include "wirelink/usb_bulk_defaults.h"

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)

static demo_endpoint_t a, b;
static wl_time_ms_t server_time;
static wl_time_ms_t test_now;
static wl_time_ms_t read_clock(void *user) { (void)user; return test_now; }
static wl_err_t configure(demo_endpoint_config_t *config, uint64_t session) {
  wl_err_t result = demo_endpoint_config_defaults(config, test_environment_id(session, (wl_clock_t){0}), wl_link_defaults());
  config->environment.clock = (wl_clock_t){read_clock, NULL};
  return result;
}
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
  result = demo_endpoint_execute_complete(context, token, &response);
  return demo_runtime_result_ok(&result) ? 0 : -1;
}

typedef struct {
  wl_ctx_t *source, *destination;
  const uint8_t *bytes;
  size_t length, offset;
  wl_io_token_t token;
} stream_t;
static stream_t streams[2];
static wl_sink_result_t stream_sink(void *context, wl_io_token_t token,
    const uint8_t *bytes, size_t length) {
  stream_t *stream = context;
  if (stream->bytes != NULL) return WL_SINK_BUSY;
  stream->bytes = bytes; stream->length = length; stream->offset = 0;
  stream->token = token;
  return WL_SINK_STARTED;
}
static int stream_service(void *context) {
  (void)context;
  for (unsigned i = 0; i < 2; ++i) {
    stream_t *stream = &streams[i];
    if (stream->bytes == NULL) continue;
    size_t accepted = 0, length = stream->length - stream->offset;
    if (length > 7) length = 7;
    int result = wl_feed_bytes(stream->destination, stream->bytes + stream->offset, length, &accepted);
    if (result != WL_OK && result != WL_ERR_NO_SPACE) return result;
    stream->offset += accepted;
    if (stream->offset == stream->length) {
      stream->bytes = NULL;
      result = wl_tx_consumed(stream->source, stream->token, WL_OK);
      if (result != WL_OK) return result;
    }
  }
  return WL_OK;
}

static wl_readiness_t stream_readiness(const void *context, const wl_ctx_t *link, wl_time_ms_t now) {
  (void)context; (void)link; (void)now;
  wl_readiness_t hint = wl_readiness_idle();
  hint.runnable = streams[0].bytes != NULL || streams[1].bytes != NULL;
  return hint;
}

static int exercise(wl_link_defaults_t defaults, wl_integrity_t integrity) {
  demo_endpoint_config_t client_config, server_config;
  wl_loopback_t cable, other;
  state_t state, read;
  alarm_t alarm, read_alarm;
  request_t request;
  const response_value_t *response;
  demo_runtime_result_t result;
  const demo_runtime_rpc_detail_t *detail;
  wl_rpc_client_result_t operation;
  wl_readiness_t hint;
  uint32_t id;

  CHECK((test_now = 0, demo_endpoint_step(&a)) == WL_ERR_NOT_INITIALIZED);
  CHECK(demo_endpoint_init(&a, test_environment_id(0, (wl_clock_t){read_clock, NULL}), wl_link_defaults()) == WL_ERR_INVALID_ARG);
  CHECK(configure(&client_config, 1) == WL_OK);
  CHECK(configure(&server_config, 2) == WL_OK);
  CHECK(client_config.link.integrity == WL_INTEGRITY_CRC16);
  CHECK(server_config.link.integrity == WL_INTEGRITY_CRC16);
  CHECK(demo_endpoint_config_defaults(&client_config, client_config.environment, defaults) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&server_config, server_config.environment, defaults) == WL_OK);
  CHECK(client_config.link.integrity == defaults.integrity);
  CHECK(server_config.link.integrity == defaults.integrity);
  if (integrity != defaults.integrity) {
    client_config.link.integrity = integrity;
    server_config.link.integrity = integrity;
  }
  calls = 0;
  CHECK(demo_runtime_config_enable_client(&client_config.advanced) == WL_OK);
  CHECK(demo_runtime_config_enable_server(&server_config.advanced) == WL_OK);
  client_config.link.ack_timeout_ms = 5;
  server_config.link.ack_timeout_ms = 5;
  server_config.advanced.execute_request_handler = execute;
  server_config.advanced.execute_user_data = &b;
  server_config.advanced.rpc_server_pending_timeout_ms = 100;
  server_config.advanced.rpc_server_cache_ttl_ms = 1000;
  CHECK(demo_endpoint_init_config(&a, &client_config) == WL_OK);
  CHECK(demo_endpoint_init_config(&b, &server_config) == WL_OK);
  wl_config_t actual;
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&a)), &actual) == WL_OK);
  CHECK(actual.integrity == integrity && actual.envelope == client_config.link.envelope);
  CHECK(demo_endpoint_init(&a, test_environment_id(7, (wl_clock_t){read_clock, NULL}), wl_link_defaults()) == WL_ERR_INVALID_STATE);
  if (client_config.link.envelope == WL_ENVELOPE_NATIVE_PACKET) {
    CHECK(wl_loopback_connect(&cable, demo_endpoint_handle(&a), demo_endpoint_handle(&b)) == WL_OK);
    CHECK(wl_loopback_connect(&other, demo_endpoint_handle(&a), demo_endpoint_handle(&b)) == WL_ERR_BUSY);
  } else {
    streams[0] = (stream_t){.source = wl_endpoint_link(demo_endpoint_handle(&a)),
      .destination = wl_endpoint_link(demo_endpoint_handle(&b))};
    streams[1] = (stream_t){.source = streams[0].destination, .destination = streams[0].source};
    CHECK(wl_set_sink(streams[0].source, stream_sink, &streams[0]) == WL_OK);
    CHECK(wl_set_sink(streams[1].source, stream_sink, &streams[1]) == WL_OK);
    const wl_pump_hooks_t hooks = {.service = stream_service, .adapter_readiness = stream_readiness};
    CHECK(wl_endpoint_attach(demo_endpoint_handle(&a), &hooks) == WL_OK);
    CHECK(wl_endpoint_attach(demo_endpoint_handle(&b), &hooks) == WL_OK);
  }

  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&a)), &actual) == WL_OK);
  CHECK(actual.integrity == integrity); /* Binding a sink never changes defaults. */

  state_clear(&state);
  state.has_sequence = true;
  for (uint32_t i = 1; i <= 2; ++i) {
    state.sequence = i;
    CHECK(demo_endpoint_send_state(&a, &state).domain == DEMO_SEND_OK);
    CHECK((test_now = i, wl_endpoint_get_readiness(demo_endpoint_handle(&a), &hint)) == WL_OK);
    CHECK(hint.runnable != 0);
    CHECK((test_now = i, demo_endpoint_step(&a)) == WL_OK);
    CHECK((test_now = i, demo_endpoint_step(&b)) == WL_OK);
  }
  for (unsigned i = 0; i < 12; ++i) { CHECK(demo_endpoint_step(&a) == WL_OK); CHECK(demo_endpoint_step(&b) == WL_OK); }
  CHECK(demo_endpoint_read_state(&b, &read) == WL_OK && read.sequence == 2);
  CHECK(demo_endpoint_read_state(&b, &read) == WL_ERR_NO_DATA && read.sequence == 2);
  CHECK(demo_endpoint_read_state(&b, NULL) == WL_ERR_INVALID_ARG);

  alarm_clear(&alarm);
  alarm.has_code = true;
  alarm.code = 17;
  CHECK(demo_endpoint_send_alarm(&a, &alarm).domain == DEMO_SEND_OK);
  CHECK((test_now = 3, demo_endpoint_step(&a)) == WL_OK);
  CHECK((test_now = 3, demo_endpoint_step(&b)) == WL_OK);
  CHECK((test_now = 4, demo_endpoint_step(&a)) == WL_OK); /* Non-RPC reliable completion. */
  for (unsigned i = 0; i < 12; ++i) { CHECK(demo_endpoint_step(&a) == WL_OK); CHECK(demo_endpoint_step(&b) == WL_OK); }
  CHECK(demo_endpoint_read_alarm(&b, &read_alarm) == WL_OK && read_alarm.code == 17);
  CHECK(demo_endpoint_read_alarm(&b, &read_alarm) == WL_ERR_NO_DATA);

  request_clear(&request);
  request.has_input = true;
  request.input = 41;
  result = (test_now = 10, demo_endpoint_execute_start(&a, &request, 100));
  detail = demo_runtime_result_rpc_detail(&result);
  CHECK(demo_runtime_result_ok(&result) && detail != NULL);
  id = detail->operation_id;
  for (server_time = 10; server_time < 30; ++server_time) {
    CHECK((test_now = server_time, demo_endpoint_step(&a)) == WL_OK);
    CHECK((test_now = server_time, demo_endpoint_step(&b)) == WL_OK);
  }
  CHECK(calls == 1);
  CHECK(demo_endpoint_execute_inspect(&a, id, &operation) == WL_RPC_OK);
  CHECK(operation.state == WL_RPC_CLIENT_COMPLETED);
  CHECK(demo_execute_client_response(demo_endpoint_runtime(&a), id, &response) == WL_RPC_OK);
  CHECK(response->output == 42);
  CHECK(demo_endpoint_execute_release(&a, id) == WL_RPC_OK);
  demo_endpoint_close(&a);
  demo_endpoint_close(&b);
  demo_endpoint_close(&a);
  CHECK((test_now = 30, demo_endpoint_step(&a)) == WL_ERR_NOT_INITIALIZED);
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
  CHECK(configure(&config, 7) == WL_OK);
  config.link.envelope = WL_ENVELOPE_COBS_STREAM;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
  packet.type = WL_PACKET_DATA;
  packet.integrity = WL_INTEGRITY_CRC16;
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
  CHECK((test_now = 1, demo_endpoint_step(&a)) == WL_ERR_INVALID_STATE);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_UNKNOWN_MESSAGE);
  CHECK(demo_endpoint_read_state(&a, &read) == WL_OK && read.sequence == 8);
  CHECK((test_now = 2, demo_endpoint_step(&a)) == WL_OK); /* Previous error does not poison a new pass. */
  demo_endpoint_close(&a);

  CHECK(configure(&config, 8) == WL_OK);
  config.link.ack_timeout_ms = 5;
  config.link.max_retries = 0; /* This case tests first-ACK timeout, not defaults. */
  CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
  test_now = 0;
  CHECK(wl_set_sink(wl_endpoint_link(demo_endpoint_handle(&a)), blackhole, NULL) == WL_OK);
  alarm_clear(&alarm);
  alarm.has_code = true;
  alarm.code = 3;
  CHECK(demo_endpoint_send_alarm(&a, &alarm).domain == DEMO_SEND_OK);
  CHECK((test_now = 1, demo_endpoint_step(&a)) == WL_OK);
  CHECK((test_now = 10, demo_endpoint_step(&a)) == WL_ERR_INVALID_STATE);
  CHECK(demo_endpoint_result(&a)->domain == DEMO_RUNTIME_CORE_ERROR);
  demo_endpoint_close(&a);
  return 0;
}

#if !WL_LINK_PROFILE_COBS_CRC16
static int mismatch(void) {
  demo_endpoint_config_t config;
  uint8_t wire[DEMO_ENDPOINT_UNIT_CAPACITY], payload[DEMO_ENDPOINT_MAX_PAYLOAD];
  size_t length, written;
  state_t value, read;
  wl_frame_view_t view;
  wl_wire_packet_t packet = {0};
  CHECK(configure(&config, 10) == WL_OK);
  CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
  state_clear(&value);
  value.has_sequence = true;
  value.sequence = 8;
  CHECK(state_encode(&value, payload, sizeof(payload), &length) == WL_CODEC_OK);
  packet.type = WL_PACKET_DATA;
  packet.integrity = WL_INTEGRITY_CRC32C;
  packet.message_id = STATE_MESSAGE_ID;
  packet.payload = payload;
  packet.payload_len = length;
  CHECK(wl_frame_encode(&packet, WL_ENVELOPE_NATIVE_PACKET, wire, sizeof(wire), &written) == WL_OK);
  CHECK(wl_frame_decode(wire, written, WL_INTEGRITY_CRC32C, &view) == WL_OK);
  int rejected = wl_frame_decode(wire, written, WL_INTEGRITY_CRC16, &view);
  CHECK(rejected != WL_OK);
  if (config.link.envelope == WL_ENVELOPE_COBS_STREAM) {
    size_t accepted;
    CHECK(wl_frame_encode(&packet, WL_ENVELOPE_COBS_STREAM, wire, sizeof(wire), &written) == WL_OK);
    CHECK(wl_feed_bytes(wl_endpoint_link(demo_endpoint_handle(&a)), wire, written, &accepted) == WL_OK);
    CHECK(accepted == written);
  } else {
    CHECK(wl_feed_unit(wl_endpoint_link(demo_endpoint_handle(&a)), wire, written) == rejected);
  }
  CHECK(demo_endpoint_step(&a) == WL_OK);
  CHECK(demo_endpoint_read_state(&a, &read) == WL_ERR_NO_DATA);
  demo_endpoint_close(&a);
  return 0;
}
#endif

static int configurations(void) {
  demo_endpoint_config_t config;
  wl_storage_requirements_t requirements;
  for (int envelope = WL_ENVELOPE_COBS_STREAM; envelope <= WL_ENVELOPE_BUS_LENGTH16; ++envelope) {
    for (int integrity = WL_INTEGRITY_NONE; integrity <= WL_INTEGRITY_CRC32C; ++integrity) {
      CHECK(configure(&config, 9) == WL_OK);
      wl_envelope_type_t default_envelope = config.link.envelope;
      config.link.envelope = envelope;
      config.link.integrity = integrity;
      wl_config_t explicit_link = config.link;
      explicit_link.session_id = 9U; /* Advanced sizing requires an identity. */
#if WL_LINK_PROFILE_COBS_CRC16
      if (envelope != WL_ENVELOPE_COBS_STREAM || integrity != WL_INTEGRITY_CRC16) {
        CHECK(wl_config_requirements(&explicit_link, &requirements) == WL_ERR_NOT_SUPPORTED);
        CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_NOT_SUPPORTED);
        continue;
      }
#endif
      CHECK(wl_config_requirements(&explicit_link, &requirements) == WL_OK);
      CHECK(requirements.tx_unit_size <= DEMO_ENDPOINT_UNIT_CAPACITY);
      CHECK(requirements.control_unit_size <= DEMO_ENDPOINT_CONTROL_CAPACITY);
      CHECK(requirements.rx_fifo_size <= DEMO_ENDPOINT_UNIT_CAPACITY);
      if (default_envelope == WL_ENVELOPE_COBS_STREAM && envelope != default_envelope) {
        CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_NOT_SUPPORTED);
      } else {
        CHECK(demo_endpoint_init_config(&a, &config) == WL_OK);
        demo_endpoint_close(&a);
      }
    }
  }
  CHECK(configure(&config, 9) == WL_OK);
  config.link.max_payload_len = DEMO_ENDPOINT_MAX_PAYLOAD + 1;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_BUF_TOO_SMALL);
  CHECK(wl_endpoint_link(demo_endpoint_handle(&a)) == NULL);
  config.link.max_payload_len = DEMO_ENDPOINT_MAX_PAYLOAD;
  config.event_budget = 0;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_INVALID_ARG);
  return 0;
}

/* One allocation-free test allocator for the generated create path. */
static _Alignas(DEMO_ENDPOINT_ALIGNMENT) uint8_t allocated[sizeof(demo_endpoint_t)];
static unsigned in_use, freed;
static void *allocate_endpoint(void *context, size_t size, size_t alignment) {
  (void)context;
  if (in_use || size != sizeof(allocated) || alignment != DEMO_ENDPOINT_ALIGNMENT) return NULL;
  in_use = 1;
  return allocated;
}
static void free_endpoint(void *context, void *pointer, size_t size, size_t alignment) {
  (void)context; (void)pointer; (void)size; (void)alignment;
  in_use = 0;
  ++freed;
}
static wl_environment_t environment(void) {
  return test_environment_id(99, (wl_clock_t){read_clock, NULL});
}
static int selected_defaults(void) {
  demo_endpoint_config_t config;
  wl_config_t actual, original = {0}, expected;
  wl_link_defaults_t invalid = {99};
  original.max_payload_len = 13;
  original.envelope = WL_ENVELOPE_COBS_STREAM;
  original.session_id = 27;
  original.max_retries = 2;
  original.ack_timeout_ms = 17;
  original.max_transmission_unit = 200;
  expected = original;
  CHECK(wl_config_apply_link_defaults(NULL, wl_link_defaults()) == WL_ERR_INVALID_ARG);
  CHECK(wl_config_apply_link_defaults(&original, invalid) == WL_ERR_INVALID_ARG);
  CHECK(memcmp(&expected, &original, sizeof(original)) == 0);
  CHECK(wl_config_apply_link_defaults(&original, wl_link_defaults()) == WL_OK);
  expected.integrity = WL_INTEGRITY_CRC16;
  CHECK(memcmp(&expected, &original, sizeof(original)) == 0);
  CHECK(wl_config_apply_link_defaults(&original, wl_usb_bulk_link_defaults()) == WL_OK);
  expected.integrity = WL_INTEGRITY_NONE;
  CHECK(memcmp(&expected, &original, sizeof(original)) == 0);
  CHECK(demo_endpoint_config_defaults(&config, environment(), invalid) == WL_ERR_INVALID_ARG);
  CHECK(demo_endpoint_init(&a, environment(), invalid) == WL_ERR_INVALID_ARG);
  CHECK(configure(&config, 99) == WL_OK);
  config.link.integrity = -1;
  CHECK(demo_endpoint_init_config(&a, &config) == WL_ERR_INVALID_ARG);

  /* Factory values and temporary descriptors have no retained identity. */
  {
    wl_link_defaults_t temporary = wl_usb_bulk_link_defaults();
    CHECK(demo_endpoint_config_defaults(&config, environment(), temporary) == WL_OK);
    temporary.integrity = WL_INTEGRITY_CRC32C;
  }
  CHECK(config.link.integrity == WL_INTEGRITY_NONE);
  const wl_allocator_t allocator = {allocate_endpoint, free_endpoint, NULL};
  demo_endpoint_t *dynamic = NULL;
#if WL_LINK_PROFILE_COBS_CRC16
  CHECK(demo_endpoint_init(&a, environment(), wl_usb_bulk_link_defaults()) == WL_ERR_NOT_SUPPORTED);
  CHECK(demo_endpoint_create(&dynamic, &config, &allocator) == WL_ERR_NOT_SUPPORTED);
  CHECK(dynamic == NULL && in_use == 0 && freed == 1);
  CHECK(config.link.integrity == WL_INTEGRITY_NONE);
#endif
  for (int integrity = WL_INTEGRITY_NONE; integrity <= WL_INTEGRITY_CRC32C; ++integrity) {
#if WL_LINK_PROFILE_COBS_CRC16
    if (integrity != WL_INTEGRITY_CRC16) continue;
#endif
    /* create accepts the final config, including explicit USB CRC overrides. */
    CHECK(demo_endpoint_config_defaults(&config, environment(), wl_usb_bulk_link_defaults()) == WL_OK);
    if (integrity != WL_INTEGRITY_NONE) config.link.integrity = integrity;
    CHECK(demo_endpoint_create(&dynamic, &config, &allocator) == WL_OK);
    CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(dynamic)), &actual) == WL_OK);
    CHECK(actual.integrity == integrity && actual.envelope == config.link.envelope);
    CHECK(demo_endpoint_close(dynamic) == WL_OK);
    CHECK(demo_endpoint_init_config(dynamic, &config) == WL_OK);
    CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(dynamic)), &actual) == WL_OK);
    CHECK(actual.integrity == integrity);
    CHECK(demo_endpoint_destroy(&dynamic) == WL_OK && dynamic == NULL && in_use == 0);
  }
  CHECK(demo_endpoint_config_defaults(&config, environment(), wl_link_defaults()) == WL_OK);
  CHECK(demo_endpoint_create(&dynamic, &config, &allocator) == WL_OK);
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(dynamic)), &actual) == WL_OK);
  CHECK(actual.integrity == WL_INTEGRITY_CRC16);
  CHECK(demo_endpoint_destroy(&dynamic) == WL_OK);
  CHECK(demo_endpoint_init(&a, environment(), wl_link_defaults()) == WL_OK);
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&a)), &actual) == WL_OK);
  CHECK(actual.integrity == WL_INTEGRITY_CRC16);
#if !WL_LINK_PROFILE_COBS_CRC16
  static demo_endpoint_t crc_usb;
  CHECK(demo_endpoint_init(&b, environment(), wl_usb_bulk_link_defaults()) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&config, environment(), wl_usb_bulk_link_defaults()) == WL_OK);
  config.link.integrity = WL_INTEGRITY_CRC32C;
  CHECK(demo_endpoint_init_config(&crc_usb, &config) == WL_OK);
  CHECK(wl_set_sink(wl_endpoint_link(demo_endpoint_handle(&b)), blackhole, NULL) == WL_OK);
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&b)), &actual) == WL_OK);
  CHECK(actual.integrity == WL_INTEGRITY_NONE);
  CHECK(demo_endpoint_close(&b) == WL_OK);
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&a)), &actual) == WL_OK);
  CHECK(actual.integrity == WL_INTEGRITY_CRC16);
  CHECK(wl_get_config(wl_endpoint_link(demo_endpoint_handle(&crc_usb)), &actual) == WL_OK);
  CHECK(actual.integrity == WL_INTEGRITY_CRC32C);
  CHECK(demo_endpoint_close(&crc_usb) == WL_OK);
#endif
  CHECK(demo_endpoint_close(&a) == WL_OK);
  return 0;
}

int main(void) {
  CHECK(selected_defaults() == 0);
  CHECK(exercise(wl_link_defaults(), WL_INTEGRITY_CRC16) == 0);
#if !WL_LINK_PROFILE_COBS_CRC16
  CHECK(exercise(wl_usb_bulk_link_defaults(), WL_INTEGRITY_NONE) == 0);
  CHECK(exercise(wl_usb_bulk_link_defaults(), WL_INTEGRITY_CRC16) == 0);
  CHECK(exercise(wl_usb_bulk_link_defaults(), WL_INTEGRITY_CRC32C) == 0);
#endif
  CHECK(failures() == 0);
#if !WL_LINK_PROFILE_COBS_CRC16
  CHECK(mismatch() == 0);
#endif
  CHECK(configurations() == 0);
  return 0;
}
