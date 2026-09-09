/* SPDX-License-Identifier: Apache-2.0 */
#include "sender_endpoint.h"
#include "receiver_endpoint.h"
#include "reliable_sender_endpoint.h"
#include "reliable_receiver_endpoint.h"
#include "test_environment.h"
#include "wirelink/loopback.h"
#include <assert.h>

static sender_endpoint_t sender;
static receiver_endpoint_t receiver;
static reliable_sender_endpoint_t reliable_sender;
static reliable_receiver_endpoint_t reliable_receiver;
static wl_loopback_t cable;
static unsigned clock_reads;
static wl_time_ms_t now(void *context) { (void)context; ++clock_reads; return 1; }

int main(void) {
  sender_runtime_config_t config;
  sender_runtime_requirements_t requirements;
  assert(sender_runtime_config_defaults(&config) == WL_OK);
  assert(sender_runtime_requirements(&config, &requirements) == WL_OK);
  assert(requirements.storage_size == 0); /* No retained or RPC storage. */
  assert(SENDER_ENDPOINT_MAX_PAYLOAD == STATE_MAX_ENCODED_SIZE);
  assert(sender_endpoint_init(&sender, test_environment_id(1, (wl_clock_t){now, NULL})) == WL_OK);
  assert(receiver_endpoint_init(&receiver, test_environment_id(2, (wl_clock_t){now, NULL})) == WL_OK);
  assert(wl_loopback_connect(&cable, sender_endpoint_handle(&sender), receiver_endpoint_handle(&receiver)) == WL_OK);
  state_t sent, received;
  state_clear(&sent);
  sent.has_data = true;
  for (unsigned i = 0; i < 100; ++i) sent.data[i] = i + 1000;
  clock_reads = 0;
  assert(sender_endpoint_send_state(&sender, &sent).domain == DEMO_SEND_OK);
  assert(clock_reads == 0);
  for (unsigned i = 0; i < 4; ++i) {
    assert(sender_endpoint_step(&sender) == WL_OK);
    assert(receiver_endpoint_step(&receiver) == WL_OK);
  }
  assert(receiver_endpoint_read_state(&receiver, &received) == WL_OK);
  assert(received.has_data && received.data[0] == 1000 && received.data[99] == 1099);
  assert(sender_endpoint_close(&sender) == WL_OK);
  assert(receiver_endpoint_close(&receiver) == WL_OK);

  assert(reliable_sender_endpoint_init(&reliable_sender, test_environment_id(3, (wl_clock_t){now, NULL})) == WL_OK);
  assert(reliable_receiver_endpoint_init(&reliable_receiver, test_environment_id(4, (wl_clock_t){now, NULL})) == WL_OK);
  assert(wl_loopback_connect(&cable, reliable_sender_endpoint_handle(&reliable_sender), reliable_receiver_endpoint_handle(&reliable_receiver)) == WL_OK);
  /* Send before the first step: only reliable admission samples the clock. */
  clock_reads = 0;
  assert(reliable_sender_endpoint_send_state(&reliable_sender, &sent).domain == DEMO_SEND_OK);
  assert(clock_reads == 1);
  for (unsigned i = 0; i < 4; ++i) {
    assert(reliable_sender_endpoint_step(&reliable_sender) == WL_OK);
    assert(reliable_receiver_endpoint_step(&reliable_receiver) == WL_OK);
  }
  assert(reliable_receiver_endpoint_read_state(&reliable_receiver, &received) == WL_OK);
  assert(received.has_data && received.data[99] == 1099);
  assert(reliable_sender_endpoint_close(&reliable_sender) == WL_OK);
  assert(reliable_receiver_endpoint_close(&reliable_receiver) == WL_OK);
  return 0;
}
