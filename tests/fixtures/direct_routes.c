/* SPDX-License-Identifier: Apache-2.0 */
#include "sender_endpoint.h"
#include "receiver_endpoint.h"
#include "test_environment.h"
#include "wirelink/loopback.h"
#include <assert.h>
#include <string.h>

static sender_endpoint_t sender;
static receiver_endpoint_t receiver;
static wl_loopback_t cable;
static unsigned calls, errors, releases, observations, closes;
static int32_t application_result;
static wl_time_ms_t now(void *context) { (void)context; return 1U; }
static int32_t chunk(void *context, const chunk_t *message, wl_delivery_t delivery) {
  assert(context == &calls);
  assert(delivery == WL_DELIVERY_RELIABLE);
  assert(message->has_data && message->data.length == 512U);
  assert(message->data.data[0] == 0x55U && message->data.data[511] == 0x55U);
  ++calls;
  assert(receiver_endpoint_step(&receiver) == WL_ERR_REENTRANT);
  return application_result;
}
static void record(void *context, const receiver_runtime_result_t *result) {
  (void)context;
  if (result->event_type == WL_EVT_RELIABLE_RX || result->event_type == WL_EVT_UNRELIABLE_RX) {
    assert(result->event_consumed);
    ++releases;
    if (result->domain != RECEIVER_RUNTIME_OK) ++errors;
  }
}
static void peer(void *context, wl_ctx_t *link, uint64_t previous, uint64_t session, wl_time_ms_t time) {
  (void)context; (void)link;
  uint64_t original = wl_link_session_id(wl_endpoint_link(sender_endpoint_handle(&sender)));
  assert(time == 1U);
  if (observations == 0U) assert(previous == 0U && session == original && calls == 0U);
  else assert(previous == original && session == original + 1U && calls == 2U);
  ++observations;
}
static void close_service(void *context, wl_ctx_t *link) {
  (void)context;
  assert(link == wl_endpoint_link(receiver_endpoint_handle(&receiver)));
  ++closes;
  wl_endpoint_close(receiver_endpoint_handle(&receiver)); /* guarded */
}
static void pump(void) {
  for (unsigned i = 0; i < 8; ++i) {
    assert(sender_endpoint_step(&sender) == WL_OK);
    wl_err_t result = receiver_endpoint_step(&receiver);
    assert(result == WL_OK || result == WL_ERR_INVALID_STATE);
  }
}
int main(void) {
  receiver_endpoint_config_t config;
  assert(RECEIVER_ENDPOINT_MAX_PAYLOAD == CHUNK_MAX_ENCODED_SIZE);
  assert(receiver_endpoint_config_defaults(&config, test_environment_id(2U, (wl_clock_t){now, NULL})) == WL_OK);
  config.on_chunk = chunk;
  config.chunk_user_data = &calls;
  config.on_result = record;
  assert(receiver_endpoint_init_config(&receiver, &config) == WL_OK);
  wl_endpoint_service_t service = {.on_peer_session = peer, .on_close = close_service};
  assert(wl_endpoint_set_services(receiver_endpoint_handle(&receiver), &service, 1U) == WL_OK);
  assert(sender_endpoint_init(&sender, test_environment_id(1U, (wl_clock_t){now, NULL})) == WL_OK);
  assert(wl_loopback_connect(&cable, sender_endpoint_handle(&sender), receiver_endpoint_handle(&receiver)) == WL_OK);
  uint8_t bytes[512];
  memset(bytes, 0x55, sizeof(bytes));
  chunk_t message;
  chunk_clear(&message);
  message.has_data = true;
  message.data.data = bytes; message.data.length = sizeof(bytes);
  assert(sender_endpoint_send_chunk(&sender, &message).domain == DEMO_SEND_OK);
  pump();
  assert(calls == 1U && releases == 1U && observations == 1U && errors == 0U);
  assert(wl_endpoint_set_services(receiver_endpoint_handle(&receiver), NULL, 0U) == WL_ERR_INVALID_STATE);
  application_result = -17;
  assert(sender_endpoint_send_chunk(&sender, &message).domain == DEMO_SEND_OK);
  pump();
  assert(calls == 2U && releases == 2U && errors == 1U);
  /* Malformed, unknown and delivery-mismatch RX must all release their lease. */
  for (unsigned i = 0; i < 3; ++i) {
    uint16_t id = i == 1U ? 99U : CHUNK_MESSAGE_ID;
    wl_ctx_t *link = wl_endpoint_link(sender_endpoint_handle(&sender));
    wl_tx_handle_t handle;
    assert((i == 2U ? wl_send_unreliable(link, id, bytes, 1U)
        : wl_send_reliable(link, id, bytes, 1U, 1U, &handle)) == WL_OK);
    pump();
  }
  assert(calls == 2U && releases == 5U && errors == 4U);
  receiver_endpoint_runtime(&receiver)->chunk_direct.handler = NULL;
  assert(sender_endpoint_send_chunk(&sender, &message).domain == DEMO_SEND_OK);
  pump();
  assert(releases == 6U && errors == 5U);
  receiver_endpoint_runtime(&receiver)->chunk_direct.handler = chunk;
  chunk_t *scratch = receiver_endpoint_runtime(&receiver)->chunk_direct.scratch;
  receiver_endpoint_runtime(&receiver)->chunk_direct.scratch = NULL;
  assert(sender_endpoint_send_chunk(&sender, &message).domain == DEMO_SEND_OK);
  pump();
  assert(releases == 7U && errors == 6U);
  receiver_endpoint_runtime(&receiver)->chunk_direct.scratch = scratch;
  /* A rebooted peer's first RX can be direct, before any RPC request. */
  uint8_t encoded[CHUNK_MAX_ENCODED_SIZE], unit[CHUNK_MAX_ENCODED_SIZE + 32U];
  size_t encoded_length, unit_length;
  assert(chunk_encode(&message, encoded, sizeof(encoded), &encoded_length) == WL_CODEC_OK);
  wl_wire_packet_t packet = {.type = WL_PACKET_DATA, .integrity = config.link.integrity,
    .flags = WL_PACKET_FLAG_RELIABLE, .message_id = CHUNK_MESSAGE_ID,
    .session_id = wl_link_session_id(wl_endpoint_link(sender_endpoint_handle(&sender))) + 1U,
    .sequence = 1U, .payload = encoded, .payload_len = encoded_length};
  assert(wl_frame_encode(&packet, WL_ENVELOPE_NATIVE_PACKET, unit, sizeof(unit), &unit_length) == WL_OK);
  assert(wl_feed_unit(wl_endpoint_link(receiver_endpoint_handle(&receiver)), unit, unit_length) == WL_OK);
  pump();
  assert(observations == 2U && calls == 3U && releases == 8U);
  assert(sender_endpoint_close(&sender) == WL_OK);
  assert(receiver_endpoint_close(&receiver) == WL_OK);
  assert(receiver_endpoint_close(&receiver) == WL_OK);
  assert(closes == 1U);
  return 0;
}
