/* SPDX-License-Identifier: Apache-2.0 */
/* Included by async_endpoint.c; uses its generated types and CHECK helper. */
static demo_endpoint_t sync_static_client, sync_server;
static demo_endpoint_t *sync_client = &sync_static_client;
static wl_loopback_t sync_cable;
static unsigned sync_waits, sync_handlers, sync_other_done, sync_steps;
static uint32_t sync_last_wait;
static bool sync_blackhole;
static int sync_wait_error;

static int sync_transfer(void *context) {
  (void)context;
  wl_loopback_service_result_t result;
  return wl_loopback_service(&sync_cable, 4, &result);
}
static int sync_service(void *context) {
  if (++sync_steps > 10000) {
    wl_poll_hint_t hint;
    wl_rpc_deadline_hint_t rpc_hint;
    wl_adapter_stats_t a, b;
    (void)wl_poll_get_hint(wl_endpoint_link(demo_endpoint_handle(sync_client)), now, &hint);
    (void)demo_runtime_get_deadline_hint(demo_endpoint_runtime(sync_client), now, &rpc_hint);
    (void)wl_loopback_get_stats(&sync_cable, WL_LOOPBACK_ENDPOINT_A, &a);
    (void)wl_loopback_get_stats(&sync_cable, WL_LOOPBACK_ENDPOINT_B, &b);
    fprintf(stderr, "sync stuck: handlers=%u waits=%u blackhole=%u wait_error=%d time=%u\n",
        sync_handlers, sync_waits, sync_blackhole, sync_wait_error, now);
    fprintf(stderr, "hints core=%u work=%u rpc=%u txa=%u txb=%u\n", hint.next_deadline_ms,
        hint.work_pending, rpc_hint.next_deadline_ms, a.tx_active, b.tx_active);
    abort();
  }
  int error = sync_transfer(context);
  if (error != WL_OK && error != WL_ERR_NO_DATA && error != WL_ERR_WOULD_BLOCK) return error;
  if (!sync_blackhole) {
    CHECK(demo_endpoint_step(&sync_server) == WL_OK);
    error = sync_transfer(context);
    CHECK(error == WL_OK || error == WL_ERR_NO_DATA || error == WL_ERR_WOULD_BLOCK);
  }
  return WL_OK;
}
static uint32_t sync_hint(const void *context, wl_time_ms_t time) {
  (void)context; (void)time;
  /* A deliberately stopped peer cannot consume its RX slot. Outstanding
   * loopback transfers are blocked, not immediately progressable work. */
  if (sync_blackhole) return UINT32_MAX;
  wl_adapter_stats_t a, b;
  wl_poll_hint_t peer;
  CHECK(wl_loopback_get_stats(&sync_cable, WL_LOOPBACK_ENDPOINT_A, &a) == WL_OK);
  CHECK(wl_loopback_get_stats(&sync_cable, WL_LOOPBACK_ENDPOINT_B, &b) == WL_OK);
  CHECK(wl_poll_get_hint(wl_endpoint_link(demo_endpoint_handle(&sync_server)), time, &peer) == WL_OK);
  return a.tx_active || b.tx_active || peer.work_pending ? 0 : peer.next_deadline_ms;
}
static wl_err_t sync_wait(void *context, uint32_t maximum_ms) {
  CHECK(context == &sync_waits);
  CHECK(maximum_ms != 0 && maximum_ms != UINT32_MAX);
  ++sync_waits;
  sync_last_wait = maximum_ms;
  if (sync_wait_error != WL_OK) return sync_wait_error;
  now += maximum_ms; /* Advance the injected protocol clock, no wall-clock wait. */
  return WL_ERR_NO_DATA;
}
static int32_t sync_execute(void *context, const request_value_t *input, response_value_t *output) {
  (void)context;
  ++sync_handlers;
  if (allocate_endpoints) {
    CHECK(demo_endpoint_destroy(&sync_client) == WL_ERR_REENTRANT);
    CHECK(sync_client != NULL && wl_fixed_pool_in_use(&endpoint_pool) == 1);
  }
  response_value_t ignored;
  wl_rpc_completion_t result = demo_endpoint_execute_sync(sync_client, input, &ignored, 100);
  CHECK(result.status == WL_RPC_FAILED && result.local_error == WL_ERR_REENTRANT);
  result = demo_endpoint_execute_sync(&sync_server, input, &ignored, 100);
  CHECK(result.status == WL_RPC_FAILED && result.local_error == WL_ERR_REENTRANT);
  if (input->input == -1) return 17;
  output->has_output = output->has_name = true;
  output->output = input->input + 1;
  output->name.length = input->name.length;
  memcpy(output->name.data, input->name.data, input->name.length);
  return 0;
}
static void sync_other(void *context, const wl_rpc_completion_t *result, const response_value_t *output) {
  (void)context; (void)output;
  CHECK(result->status == WL_RPC_CANCELLED);
  ++sync_other_done;
  if (allocate_endpoints) {
    CHECK(demo_endpoint_destroy(&sync_client) == WL_ERR_REENTRANT);
    CHECK(sync_client != NULL && wl_fixed_pool_in_use(&endpoint_pool) == 1);
  }
}
static void sync_initialize(void) {
  demo_endpoint_config_t config;
  if (allocate_endpoints) {
    wl_allocator_t allocator = endpoint_allocator();
    sync_client = NULL;
    CHECK(demo_endpoint_config_defaults(&config, test_environment_id(31, (wl_clock_t){0})) == WL_OK);
    config.environment.clock = (wl_clock_t){read_clock, NULL};
    CHECK(demo_endpoint_create(&sync_client, &config, &allocator) == WL_OK);
  } else CHECK(demo_endpoint_init(sync_client, test_environment_id(31, (wl_clock_t){read_clock, NULL})) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&config, test_environment_id(32, (wl_clock_t){0})) == WL_OK);
  config.environment.clock = (wl_clock_t){read_clock, NULL};
  config.on_execute = sync_execute;
  config.on_download = download;
  CHECK(demo_endpoint_init_config(&sync_server, &config) == WL_OK);
  CHECK(wl_loopback_init(&sync_cable, wl_endpoint_link(demo_endpoint_handle(sync_client)),
      wl_endpoint_link(demo_endpoint_handle(&sync_server))) == WL_OK);
  wl_pump_hooks_t hooks = {0};
  hooks.service = sync_transfer;
  hooks.adapter_deadline_hint = sync_hint;
  CHECK(wl_endpoint_attach(demo_endpoint_handle(&sync_server), &hooks) == WL_OK);
  hooks.service = sync_service;
  CHECK(wl_endpoint_attach(demo_endpoint_handle(sync_client), &hooks) == WL_OK);
  const wl_waiter_t waiter = {sync_wait, &sync_waits, NULL};
  CHECK(wl_endpoint_set_waiter(demo_endpoint_handle(sync_client), &waiter) == WL_OK);
  sync_blackhole = false;
  sync_wait_error = WL_OK;
  sync_waits = sync_handlers = sync_other_done = sync_steps = 0;
}
static void sync_close(void) {
  wl_loopback_quiesce(&sync_cable);
  if (allocate_endpoints) {
    CHECK(demo_endpoint_destroy(&sync_client) == WL_OK && sync_client == NULL);
    CHECK(wl_fixed_pool_in_use(&endpoint_pool) == 0);
    sync_client = &sync_static_client;
  } else CHECK(demo_endpoint_close(sync_client) == WL_OK);
  CHECK(demo_endpoint_close(&sync_server) == WL_OK);
}
static void run_sync_tests(void) {
  request_value_t value = request();
  response_value_t response, unchanged;
  wl_rpc_completion_t result;
  memset(&response, 0xa5, sizeof(response));
  unchanged = response;
  sync_initialize();
  unsigned before_allocations = allocations, before_deallocations = deallocations;
  CHECK(wl_endpoint_set_waiter(demo_endpoint_handle(sync_client), NULL) == WL_OK);
  result = demo_endpoint_execute_sync(sync_client, &value, &response, 100);
  CHECK(result.status == WL_RPC_FAILED && result.local_error == WL_ERR_NOT_SUPPORTED);
  CHECK(memcmp(&response, &unchanged, sizeof(response)) == 0 && sync_handlers == 0);
  const wl_waiter_t waiter = {sync_wait, &sync_waits, NULL};
  CHECK(wl_endpoint_set_waiter(demo_endpoint_handle(sync_client), &waiter) == WL_OK);
  for (unsigned i = 0; i < 100; ++i) {
    result = demo_endpoint_execute_sync(sync_client, &value, &response, 100);
    CHECK(result.status == WL_RPC_SUCCESS && result.local_error == WL_OK);
    CHECK(response.output == 42 && response.name.length == 3 && memcmp(response.name.data, "abc", 3) == 0);
  }
  unchanged = response;
  value.input = -1;
  result = demo_endpoint_execute_sync(sync_client, &value, &response, 100);
  CHECK(result.status == WL_RPC_REJECTED && result.rejection == 17 && result.local_error == WL_OK);
  CHECK(memcmp(&response, &unchanged, sizeof(response)) == 0);
  value = request();
  empty_value_t empty;
  large_value_t large;
  empty_value_clear(&empty);
  result = demo_endpoint_download_sync(sync_client, &empty, &large, 100);
  CHECK(result.status == WL_RPC_SUCCESS && large.data.length == 2023 && large.data.data[2022] == 0xa5);
  result = demo_endpoint_execute_sync(sync_client, &value, &response, 0);
  CHECK(result.status == WL_RPC_FAILED && result.local_error == WL_ERR_INVALID_ARG);
  CHECK(allocations == before_allocations && deallocations == before_deallocations);
  sync_close();
  CHECK(response.output == 42 && large.data.data[2022] == 0xa5);

  for (unsigned attempt = 0; attempt < 3; ++attempt) {
    sync_initialize();
    sync_blackhole = true;
    now = UINT32_MAX - 5U;
    if (attempt != 0) sync_wait_error = attempt == 1 ? WL_ERR_IO : WL_ERR_CANCELLED;
    if (DEMO_ENDPOINT_RPC_CAPACITY > 1 && attempt != 0)
      CHECK(demo_endpoint_execute_async(sync_client, &value, 1000, sync_other, NULL, NULL) == WL_OK);
    result = demo_endpoint_execute_sync(sync_client, &value, &response, 10);
    CHECK(result.status == (attempt == 0 ? WL_RPC_TIMED_OUT : attempt == 1 ? WL_RPC_FAILED : WL_RPC_CANCELLED));
    CHECK(result.local_error == sync_wait_error && sync_waits == 1 && sync_last_wait == 10);
    CHECK(sync_handlers == 0 && sync_other_done == 0);
    CHECK(memcmp(&response, &unchanged, sizeof(response)) == 0);
    /* Drive after return to expose an escaped stack callback under ASan. */
    CHECK(demo_endpoint_step(sync_client) == WL_OK);
    sync_close();
    CHECK(sync_other_done == ((DEMO_ENDPOINT_RPC_CAPACITY > 1 && attempt != 0) ? 1U : 0U));
  }
  result = demo_endpoint_execute_sync(sync_client, &value, &response, 10);
  CHECK(result.status == WL_RPC_FAILED && result.local_error == WL_ERR_NOT_INITIALIZED);
  printf("sync endpoint: capacity=%u ownership/deadlines/wait cleanup PASS\n", (unsigned)DEMO_ENDPOINT_RPC_CAPACITY);
}
