/* SPDX-License-Identifier: Apache-2.0 */
#include "device.h"
#include "endpoint.h"
#include <assert.h>
#include <string.h>

static uint8_t sent[2052];
static size_t sent_length;
static unsigned completed;
static wl_rpc_result_t completion;

static wl_packet_submit_t submit(wl_transport_t *user, const uint8_t *data, size_t length,
                                 wl_packet_token_t token) {
  (void)user;
  (void)token;
  assert(length <= sizeof(sent));
  memcpy(sent, data, length);
  sent_length = length;
  return WL_PACKET_SENT;
}

static wl_err_t attach(wl_transport_t *self, const wl_transport_attach_t *info) {
  (void)self;
  assert(info->rx_wire_max == DEVICE_RX_WIRE_MAX && info->tx_wire_max == DEVICE_TX_WIRE_MAX);
  return WL_OK;
}

static wl_err_t stop(wl_transport_t *self) {
  (void)self;
  return WL_OK;
}

static void done(void *context, wl_rpc_result_t result) {
  assert(context == &completed);
  ++completed;
  completion = result;
}

int main(void) {
  device_node_t device;
  static const wl_transport_ops_t ops = {.submit = submit, .attach = attach, .stop = stop};
  wl_transport_t transport = {.ops = &ops};
  assert(device_bind_instance(&device, 901U, &transport) == WL_OK);
  info_request_t request = {0};
  info_response_t response;
  memset(&response, 0xA5, sizeof(response));
  device_sdk_call_t call = {.endpoint = &device.node, .request = &request, .response = &response};
  wl_call_handle_t handle;
  assert(device_sdk_get_info_submit(&call, 7000U, 50U, done, &completed, &handle) == WL_OK);
  assert(wl_node_step(&device.node, 7000U) == WL_OK);
  wl_wire_frame_t frame;
  wl_rpc_metadata_t metadata;
  assert(wl_wire_frame_read(sent, sent_length, &frame) == WL_OK);
  assert(wl_wire_rpc_read(frame.kind, frame.payload, frame.length, &metadata) == WL_OK);
  uint8_t reply[2052];
  size_t header_size = 0U, body_size = 0U;
  assert(wl_wire_rpc_write(reply + 4U, sizeof(reply) - 4U, metadata.session,
                           metadata.operation, 0, &header_size) == WL_OK);
  info_response_t value;
  info_response_clear(&value);
  value.settings.enabled = true;
  assert(info_response_encode(&value, reply + 4U + header_size,
                               sizeof(reply) - 4U - header_size, &body_size) == WL_CODEC_OK);
  assert(wl_wire_frame_seal(reply, sizeof(reply), WL_WIRE_RPC_RESP,
                            INFO_RESPONSE_MESSAGE_ID, header_size + body_size) == WL_OK);
  assert(wl_node_rx_packet(&device.node, reply, 4U + header_size + body_size) == WL_OK);
  assert(wl_node_step(&device.node, 7001U) == WL_OK);
  assert(completed == 1U && completion.outcome == WL_RPC_SUCCESS && response.settings.enabled);

  /* Explicit SDK time stays authoritative after a long owner idle period. */
  assert(device_sdk_get_info_submit(&call, 100000U, 50U, done, &completed, NULL) == WL_OK);
  assert(wl_node_step(&device.node, 100000U) == WL_OK);
  assert(completed == 1U);
  assert(wl_node_step(&device.node, 100050U) == WL_OK);
  assert(completed == 2U && completion.outcome == WL_RPC_TIMED_OUT);
  assert(wl_node_close(&device.node, 100051U) == WL_OK);
  assert(wl_node_step(&device.node, 100051U) == WL_OK);
  assert(wl_node_quiescent(&device.node));
  return 0;
}
