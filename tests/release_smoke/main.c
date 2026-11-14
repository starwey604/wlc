/* SPDX-License-Identifier: Apache-2.0 */
#include "release_smoke_kernel.h"
#include <wirelink/packet_loopback.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static release_smoke_kernel_t endpoints[2];
static wl_packet_port_t ports[2];
static uint8_t storage[2][2][256];
static wl_packet_loopback_t transport;
static unsigned completions, executions;
static void require(int condition) {
  if (!condition) {
    fputs("archived compiler RPC smoke failed\n", stderr);
    exit(EXIT_FAILURE);
  }
}
static int32_t apply(void *user, const control_t *request, control_result_value_t *response) {
  (void)user;
  require(request->has_operation_id && request->operation_id == UINT32_MAX);
  require(request->has_joints && request->joints[0] == -1.5f && request->joints[5] == 3.25f);
  response->has_operation_id = response->has_result = true;
  response->operation_id = request->operation_id;
  response->result = OK;
  ++executions;
  return 0;
}
static void completed(void *user, wl_kernel_result_t result, const control_result_t *response) {
  (void)user;
  require(result.outcome == WL_KERNEL_SUCCESS && response != NULL);
  require(response->has_operation_id && response->operation_id == UINT32_MAX);
  require(response->has_result && response->result == OK);
  ++completions;
}
int main(void) {
  require(wl_packet_loopback_init(&transport, &ports[0], &ports[1], NULL, NULL) == WL_OK);
  for (unsigned side = 0; side < 2U; ++side) {
    wl_packet_config_t config = {.instance = 1001U + side,
        .rx_storage = &storage[side][0][0], .rx_unit_size = 256U,
        .rx_storage_size = sizeof(storage[side]), .submit = wl_packet_loopback_submit,
        .notify = wl_packet_loopback_notify, .user = &transport.directions[side]};
    require(wl_packet_init(&ports[side], &config) == WL_OK);
    release_smoke_kernel_handlers_t handlers = {.on_rpc_apply_control = apply};
    require(release_smoke_kernel_init(&endpoints[side], &ports[side], &handlers, NULL, NULL) == WL_OK);
    control_value_t request = {.has_operation_id = true, .operation_id = UINT32_MAX,
        .has_joints = true, .joints = {-1.5f, 0.0f, 1.0f, 2.0f, -0.5f, 3.25f}};
    wl_kernel_call_handle_t call;
    require(release_smoke_kernel_apply_control_start(&endpoints[side], &request, 0U, 500U,
        completed, NULL, &call) == WL_OK);
    memset(&request, 0, sizeof(request));
  }
  for (uint32_t now = 0; now < 500U && completions != 2U; ++now) {
    require(release_smoke_kernel_step(&endpoints[0], now) == WL_OK);
    require(release_smoke_kernel_step(&endpoints[1], now) == WL_OK);
    require(wl_packet_loopback_service(&transport, 2U) == WL_OK);
  }
  require(completions == 2U && executions == 2U);
  for (unsigned side = 0; side < 2U; ++side)
    require(wl_kernel_close(&endpoints[side].core) == WL_OK);
  wl_packet_loopback_stop(&transport);
  for (unsigned side = 0; side < 2U; ++side) {
    require(release_smoke_kernel_step(&endpoints[side], 500U) == WL_OK);
    require(wl_kernel_quiescent(&endpoints[side].core));
  }
  puts("archived compiler: bidirectional profile-v2 packed-float RPC PASS");
  return 0;
}
