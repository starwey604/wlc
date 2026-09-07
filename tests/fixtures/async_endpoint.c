/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.h"
#include "wirelink/loopback.h"
#include <stdio.h>
#include <stdlib.h>

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s (time=%u)\n", __LINE__, #x, now); abort(); } } while (0)
static demo_endpoint_t client, server;
static wl_loopback_t cable;
static wl_time_ms_t now;
static unsigned clock_reads, completions, handlers, chained;
static bool closing;
static wl_rpc_status_t expected = WL_RPC_SUCCESS;
static response_value_t saved;
static large_value_t large_saved;
static wl_rpc_call_t handle;
static wl_time_ms_t read_clock(void *context) {
  (void)context;
  ++clock_reads;
  return now;
}
static request_value_t request(void) {
  request_value_t value;
  request_value_clear(&value);
  value.has_input = true;
  value.input = 41;
  value.has_name = true;
  value.name.length = 3;
  memcpy(value.name.data, "abc", 3);
  return value;
}
static int32_t execute(void *context, const request_value_t *input, response_value_t *output) {
  CHECK(context == &handlers);
  ++handlers;
  CHECK(input->name.length == 3 && memcmp(input->name.data, "abc", 3) == 0);
  if (input->input == -1) return 17;
  output->has_output = true;
  output->output = input->input + 1;
  output->has_name = true;
  output->name.length = input->name.length;
  memcpy(output->name.data, input->name.data, input->name.length);
  return 0;
}
static int32_t download(void *context, const empty_value_t *input, large_value_t *output) {
  (void)context; (void)input;
  output->has_data = true;
  output->data.length = sizeof(output->data.data);
  memset(output->data.data, 0xa5, output->data.length);
  return 0;
}
static void done(void *context, const wl_rpc_completion_t *result, const response_value_t *output) {
  unsigned before = clock_reads;
  CHECK(context == &completions);
  CHECK(result->status == expected);
  ++completions;
  if (result->status == WL_RPC_SUCCESS) {
    CHECK(output != NULL && output->output == 42);
    CHECK(output->name.length == 3 && memcmp(output->name.data, "abc", 3) == 0);
    saved = *output;
  } else {
    CHECK(output == NULL);
    if (result->status == WL_RPC_REJECTED) CHECK(result->rejection == 17);
  }
  CHECK(demo_endpoint_close(&client) == WL_ERR_REENTRANT);
  CHECK(demo_endpoint_step(&client) == WL_ERR_REENTRANT);
  if (closing) {
    request_value_t value = request();
    CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_ERR_NOT_INITIALIZED);
    CHECK(demo_endpoint_init(&client, 99, (wl_clock_t){read_clock, NULL}) == WL_ERR_REENTRANT);
  }
  if (chained != 0) {
    request_value_t value = request();
    --chained;
    CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, &handle) == WL_OK);
  }
  CHECK(clock_reads == before); /* callback reuses the owner pass clock */
}
static void download_done(void *context, const wl_rpc_completion_t *result, const large_value_t *output) {
  (void)context;
  CHECK(result->status == WL_RPC_SUCCESS && output != NULL);
  CHECK(output->data.length == 2031 && output->data.data[2030] == 0xa5);
  large_saved = *output;
  ++completions;
}
static void initialize(void) {
  demo_endpoint_config_t config;
  CHECK(demo_endpoint_init(&client, 1, (wl_clock_t){read_clock, NULL}) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&config, 2) == WL_OK);
  config.clock = (wl_clock_t){read_clock, NULL};
  config.on_execute = execute;
  config.execute_user_data = &handlers;
  config.on_download = download;
  CHECK(demo_endpoint_init_config(&server, &config) == WL_OK);
  CHECK(wl_loopback_connect(&cable, demo_endpoint_handle(&client), demo_endpoint_handle(&server)) == WL_OK);
}
static void step(void) {
  unsigned before = clock_reads;
  CHECK(demo_endpoint_step(&client) == WL_OK);
  CHECK(demo_endpoint_step(&server) == WL_OK);
  CHECK(clock_reads == before + 2);
  ++now;
}
static void wait_for(unsigned target) {
  for (unsigned i = 0; i < 300 && completions != target; ++i) step();
  CHECK(completions == target);
  for (unsigned i = 0; i < 5; ++i) step(); /* independently retire link leases */
}
static void close_pair(void) {
  unsigned before = clock_reads;
  closing = true;
  CHECK(demo_endpoint_close(&client) == WL_OK);
  CHECK(demo_endpoint_close(&server) == WL_OK);
  CHECK(demo_endpoint_close(&client) == WL_OK);
  closing = false;
  CHECK(clock_reads == before);
}
#include "sync_endpoint.c"

int main(void) {
  request_value_t value = request();
  unsigned before, target;
  initialize();
  /* First call needs no preparatory step. Request is already a snapshot. */
  before = clock_reads;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, &handle) == WL_OK);
  CHECK(clock_reads == before + 1 && completions == 0);
  memset(&value, 0xcd, sizeof(value));
  wait_for(1);
  CHECK(demo_endpoint_cancel(&client, &handle) != WL_OK);

  /* More than default cache capacity, without waiting for 10-second TTL. */
  value = request();
  for (unsigned i = 0; i < 100; ++i) {
    target = completions + 1;
    CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_OK);
    wait_for(target);
  }
  CHECK(handlers == 101);
  target = completions + DEMO_ENDPOINT_RPC_CAPACITY;
  for (unsigned i = 0; i < DEMO_ENDPOINT_RPC_CAPACITY; ++i)
    CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_OK);
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_ERR_BUSY);
  wait_for(target);

  /* Callback can replenish a one-slot endpoint: the old call is already free. */
  target = completions + 21;
  chained = 20;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_OK);
  wait_for(target);
  expected = WL_RPC_REJECTED;
  value.input = -1;
  target = completions + 1;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, NULL) == WL_OK);
  wait_for(target);
  expected = WL_RPC_SUCCESS;
  {
    empty_value_t empty;
    empty_value_clear(&empty);
    target = completions + 1;
    CHECK(demo_endpoint_download_async(&client, &empty, 100, download_done, NULL, NULL) == WL_OK);
    wait_for(target);
  }
  /* Rejected admission never has a completion, nor overwrites cancellation authority. */
  value = request();
  before = completions;
  CHECK(demo_endpoint_execute_async(&client, &value, 0, done, &completions, &handle) == WL_ERR_INVALID_ARG);
  value.name.length = 32;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, &handle) != WL_OK);
  CHECK(completions == before);
  value = request();
  expected = WL_RPC_CANCELLED;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, &handle) == WL_OK);
  CHECK(demo_endpoint_cancel(&client, &handle) == WL_OK);
  wait_for(before + 1);
  /* Wrap-safe queue deadlines include time before any service pass. */
  now = UINT32_MAX - 5U;
  target = completions + DEMO_ENDPOINT_RPC_CAPACITY;
  expected = WL_RPC_TIMED_OUT;
  for (unsigned i = 0; i < DEMO_ENDPOINT_RPC_CAPACITY; ++i)
    CHECK(demo_endpoint_execute_async(&client, &value, 10, done, &completions, NULL) == WL_OK);
  now += 11U;
  wait_for(target);
  /* Close notifies accepted calls once; copied business data survives reuse. */
  expected = WL_RPC_CANCELLED;
  before = completions;
  CHECK(demo_endpoint_execute_async(&client, &value, 100, done, &completions, &handle) == WL_OK);
  close_pair();
  CHECK(completions == before + 1);
  initialize();
  CHECK(demo_endpoint_cancel(&client, &handle) != WL_OK);
  CHECK(saved.output == 42 && memcmp(saved.name.data, "abc", 3) == 0);
  CHECK(large_saved.data.length == 2031 && large_saved.data.data[2030] == 0xa5);
  close_pair();
  printf("async endpoint: capacity=%u bytes=%zu completions=%u handlers=%u\n",
      (unsigned)DEMO_ENDPOINT_RPC_CAPACITY, sizeof(client), completions, handlers);
  run_sync_tests();
  return 0;
}
