/* SPDX-License-Identifier: Apache-2.0 */
#include "client_advanced.h"
#include "server_advanced.h"
#include "both_endpoint.h"
#include "flex_endpoint.h"
#include <wirelink/port.h>
#include <assert.h>
#include <stdio.h>

static client_endpoint_t client;
static server_endpoint_t server;
static both_endpoint_t both;
static flex_endpoint_t flex;
static wl_time_ms_t tick;
static uint64_t sessions;
static unsigned completed;
static wl_rpc_status_t expected;
static wl_time_ms_t now(void *context) { (void)context; return tick; }
static wl_err_t session(void *context, uint64_t *out) {
  (void)context; *out = ++sessions; return WL_OK;
}
static wl_environment_t environment(void) {
  wl_environment_t env = {{now, NULL}, {session, NULL}};
  return env;
}

/* Test-only asynchronous transport. Fragment stream ingress deliberately;
 * retain TX leases until all bytes are accepted. No application callbacks here. */
typedef struct {
  wl_ctx_t *source, *destination;
  const uint8_t *data;
  size_t length, offset;
  wl_io_token_t token;
  bool stream;
} direction_t;
static direction_t directions[2];
static wl_sink_result_t sink(void *context, wl_io_token_t token, const uint8_t *data, size_t length) {
  direction_t *d = context;
  if (d->data != NULL) return WL_SINK_BUSY;
  d->data = data; d->length = length; d->offset = 0; d->token = token;
  return WL_SINK_STARTED;
}
static void transfer(direction_t *d) {
  if (d->data == NULL) return;
  int result;
  if (d->stream) {
    size_t accepted = 0;
    size_t length = d->length - d->offset;
    if (length > 17) length = 17;
    result = wl_feed_bytes(d->destination, d->data + d->offset, length, &accepted);
    d->offset += accepted;
  } else {
    result = wl_feed_unit(d->destination, d->data, d->length);
    if (result == WL_OK) d->offset = d->length;
  }
  assert(result == WL_OK || result == WL_ERR_WOULD_BLOCK || result == WL_ERR_NO_SPACE);
  if (d->offset == d->length) {
    d->data = NULL;
    assert(wl_tx_complete(d->source, d->token, WL_OK) == WL_OK);
  }
}
static void step(void) {
  transfer(&directions[0]); transfer(&directions[1]);
  assert(client_endpoint_step(&client) == WL_OK);
  assert(server_endpoint_step(&server) == WL_OK);
  ++tick;
}
static int32_t echo(void *context, const request_value_t *request, response_value_t *response) {
  (void)context;
  assert(request->has_data && request->data.length == 31);
  response->has_data = true;
  response->data.length = sizeof(response->data.data);
  memset(response->data.data, request->data.data[0], response->data.length);
  return 0;
}
static void done(void *context, const wl_rpc_completion_t *completion, const response_value_t *response) {
  (void)context;
  assert(completion->status == expected);
  if (expected == WL_RPC_SUCCESS) {
    assert(response != NULL && response->has_data && response->data.length == 1970);
    assert(response->data.data[0] == 0xa5 && response->data.data[1969] == 0xa5);
  } else assert(response == NULL);
  ++completed;
}
static void initialize(void) {
  client_endpoint_config_t c;
  server_endpoint_config_t s;
  assert(client_endpoint_config_defaults(&c, environment()) == WL_OK);
  assert(server_endpoint_config_defaults(&s, environment()) == WL_OK);
  s.on_echo = echo;
  /* Profile mismatches fail before session allocation or incarnation mutation. */
  int32_t envelope = c.link.envelope;
  uint64_t before = sessions;
  c.link.envelope = envelope == WL_ENVELOPE_NATIVE_PACKET ? WL_ENVELOPE_COBS_STREAM : WL_ENVELOPE_NATIVE_PACKET;
  assert(client_endpoint_init_config(&client, &c) == WL_ERR_NOT_SUPPORTED);
  c.link.envelope = envelope;
  assert(client_runtime_config_enable_server(&c.advanced) == WL_ERR_NOT_SUPPORTED);
  c.advanced.rpc_server_enabled = 1;
  client_runtime_requirements_t requirements;
  assert(client_runtime_requirements(&c.advanced, &requirements) == WL_ERR_NOT_SUPPORTED);
  client_runtime_init_diagnostic_t diagnostic;
  client_runtime_storage_t storage = {client.private_state.arena.bytes, sizeof(client.private_state.arena.bytes)};
  assert(client_runtime_init_checked(&client.private_state.instance, &c.advanced, &storage, &diagnostic) == WL_ERR_NOT_SUPPORTED);
  assert(diagnostic.issue == CLIENT_RUNTIME_INIT_ROLE_ENABLE);
  assert(client_endpoint_init_config(&client, &c) == WL_ERR_NOT_SUPPORTED);
  c.advanced.rpc_server_enabled = 0;
  assert(server_runtime_config_enable_client(&s.advanced) == WL_ERR_NOT_SUPPORTED);
  s.advanced.rpc_client_enabled = 1;
  assert(server_endpoint_init_config(&server, &s) == WL_ERR_NOT_SUPPORTED);
  s.advanced.rpc_client_enabled = 0;
  s.advanced.rpc_server_pending_slot_count = SERVER_ENDPOINT_RPC_CAPACITY + 1;
  assert(server_endpoint_init_config(&server, &s) == WL_ERR_INVALID_ARG);
  s.advanced.rpc_server_pending_slot_count = SERVER_ENDPOINT_RPC_CAPACITY;
  assert(sessions == before);
  assert(client_endpoint_init_config(&client, &c) == WL_OK);
  assert(server_endpoint_init_config(&server, &s) == WL_OK);
  assert(client_endpoint_runtime(&client)->rpc_server == NULL);
  assert(server_endpoint_runtime(&server)->rpc_client == NULL);
  assert(server_endpoint_runtime(&server)->rpc_async == NULL);
  wl_ctx_t *a = wl_endpoint_link(client_endpoint_handle(&client));
  wl_ctx_t *b = wl_endpoint_link(server_endpoint_handle(&server));
  directions[0] = (direction_t){a, b, NULL, 0, 0, 0, envelope == WL_ENVELOPE_COBS_STREAM};
  directions[1] = (direction_t){b, a, NULL, 0, 0, 0, envelope == WL_ENVELOPE_COBS_STREAM};
  assert(wl_set_sink(a, sink, &directions[0]) == WL_OK);
  assert(wl_set_sink(b, sink, &directions[1]) == WL_OK);
}
static void close_pair(void) {
  /* Quiesce this test adapter before releasing its endpoint storage. */
  for (unsigned i = 0; i < 2; ++i) {
    direction_t *d = &directions[i];
    if (d->data != NULL) {
      d->data = NULL;
      assert(wl_tx_complete(d->source, d->token, WL_ERR_IO) == WL_OK);
    }
    assert(wl_set_sink(d->source, NULL, NULL) == WL_OK);
  }
  assert(client_endpoint_close(&client) == WL_OK);
  assert(server_endpoint_close(&server) == WL_OK);
}

static void omitted_role_diagnostics(void) {
  /* Synthetic events have no RX lease. Normal traffic below tests real RX
   * ownership; here verify the precise diagnostics before any payload decode. */
  wl_event_t event = {0};
  event.type = WL_EVT_RELIABLE_RX;
  event.message_id = REQUEST_MESSAGE_ID;
  client_runtime_result_t c = client_runtime_dispatch_event(
      wl_endpoint_link(client_endpoint_handle(&client)), &event,
      client_endpoint_runtime(&client), tick);
  assert(c.domain == CLIENT_RUNTIME_MISSING_ROUTE);
  assert(c.detail_kind == CLIENT_RUNTIME_DETAIL_RPC && c.event_consumed == 1);
  assert(c.detail.rpc.operation_id == 0);
  event.type = WL_EVT_UNRELIABLE_RX;
  c = client_runtime_dispatch_event(wl_endpoint_link(client_endpoint_handle(&client)),
      &event, client_endpoint_runtime(&client), tick);
  assert(c.domain == CLIENT_RUNTIME_DELIVERY_MISMATCH && c.event_consumed == 1);
  event.message_id = RESPONSE_MESSAGE_ID;
  server_runtime_result_t s = server_runtime_dispatch_event(
      wl_endpoint_link(server_endpoint_handle(&server)), &event,
      server_endpoint_runtime(&server), tick);
  assert(s.domain == SERVER_RUNTIME_DELIVERY_MISMATCH && s.event_consumed == 1);
  event.type = WL_EVT_RELIABLE_RX;
  s = server_runtime_dispatch_event(wl_endpoint_link(server_endpoint_handle(&server)),
      &event, server_endpoint_runtime(&server), tick);
  assert(s.domain == SERVER_RUNTIME_MISSING_ROUTE);
  assert(s.detail_kind == SERVER_RUNTIME_DETAIL_RPC && s.event_consumed == 1);
  assert(s.detail.rpc.operation_id == 0);
}
int main(void) {
  assert(sizeof(client) < sizeof(both));
  assert(sizeof(server) < sizeof(both));
  assert(sizeof(both) <= sizeof(flex));
  printf("endpoint client=%zu server=%zu both=%zu any/both=%zu; arena=%zu/%zu/%zu; fifo=%u\n",
      sizeof(client), sizeof(server), sizeof(both), sizeof(flex),
      sizeof(client.private_state.arena), sizeof(server.private_state.arena), sizeof(both.private_state.arena), (unsigned)CLIENT_ENDPOINT_RX_FIFO_CAPACITY);
  /* Exact transport buffers must also accept the two smaller CRC choices. */
  for (int integrity = WL_INTEGRITY_NONE; integrity <= WL_INTEGRITY_CRC32C; ++integrity) {
    both_endpoint_config_t b;
    flex_endpoint_config_t f;
    assert(both_endpoint_config_defaults(&b, environment()) == WL_OK);
    assert(flex_endpoint_config_defaults(&f, environment()) == WL_OK);
    b.on_echo = echo;
    f.on_echo = echo;
    b.link.integrity = f.link.integrity = integrity;
    f.link.envelope = b.link.envelope;
    assert(both_endpoint_init_config(&both, &b) == WL_OK);
    assert(flex_endpoint_init_config(&flex, &f) == WL_OK);
    assert(both_endpoint_close(&both) == WL_OK);
    assert(flex_endpoint_close(&flex) == WL_OK);
  }
  for (unsigned pass = 0; pass < 2; ++pass) {
    initialize();
    omitted_role_diagnostics();
    expected = WL_RPC_SUCCESS;
    request_value_t request;
    request_value_clear(&request);
    request.has_data = true; request.data.length = 31;
    memset(request.data.data, 0xa5, 31);
    for (unsigned round = 0; round < CLIENT_ENDPOINT_RPC_CAPACITY * 3; ++round) {
      unsigned target = completed + 1;
      assert(client_endpoint_echo_async(&client, &request, 3000, done, NULL, NULL) == WL_OK);
      for (unsigned i = 0; i < 2500 && completed < target; ++i) step();
      assert(completed == target);
      for (unsigned i = 0; i < 30; ++i) step();
      state_t sent = {0}, received = {0};
      sent.has_seq = true; sent.seq = round + 1;
      assert(server_endpoint_send_state(&server, &sent).domain == BUSINESS_SEND_OK);
      for (unsigned i = 0; i < 30; ++i) step();
      assert(client_endpoint_read_state(&client, &received) == WL_OK);
      assert(received.has_seq && received.seq == sent.seq);
    }
    /* Close cancels queued calls exactly once, and reinit uses fresh storage. */
    expected = WL_RPC_CANCELLED;
    unsigned target = completed + CLIENT_ENDPOINT_RPC_CAPACITY;
    for (unsigned i = 0; i < CLIENT_ENDPOINT_RPC_CAPACITY; ++i)
      assert(client_endpoint_echo_async(&client, &request, 3000, done, NULL, NULL) == WL_OK);
    assert(client_endpoint_echo_async(&client, &request, 3000, done, NULL, NULL) == WL_ERR_BUSY);
    close_pair();
    assert(completed == target);
  }
  return 0;
}
