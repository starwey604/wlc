/* SPDX-License-Identifier: Apache-2.0 */
/* Included before sync_endpoint.c; the latter replays all tests in this pool. */
#include "wirelink/storage/fixed_pool.h"
#include "test_environment.h"
static _Alignas(DEMO_ENDPOINT_ALIGNMENT) unsigned char endpoint_memory[sizeof(demo_endpoint_t)];
static wl_fixed_pool_t endpoint_pool;
static unsigned allocations, deallocations;
static bool allocate_endpoints, fail_allocation, misalign_allocation;
static void *endpoint_allocate(void *context, size_t size, size_t alignment) {
  CHECK(context == &endpoint_pool && size == sizeof(demo_endpoint_t) && alignment == DEMO_ENDPOINT_ALIGNMENT);
  ++allocations;
  if (fail_allocation) return NULL;
  wl_allocator_t backing = wl_fixed_pool_allocator(&endpoint_pool);
  void *pointer = backing.allocate(backing.context, size, alignment);
  return pointer != NULL && misalign_allocation ? (unsigned char *)pointer + 1 : pointer;
}
static void endpoint_deallocate(void *context, void *pointer, size_t size, size_t alignment) {
  CHECK(context == &endpoint_pool && size == sizeof(demo_endpoint_t) && alignment == DEMO_ENDPOINT_ALIGNMENT);
  ++deallocations;
  if (misalign_allocation) pointer = (unsigned char *)pointer - 1;
  memset(pointer, 0xdd, size);
  wl_allocator_t backing = wl_fixed_pool_allocator(&endpoint_pool);
  backing.deallocate(backing.context, pointer, size, alignment);
}
static wl_allocator_t endpoint_allocator(void) {
  wl_allocator_t allocator = {endpoint_allocate, endpoint_deallocate, &endpoint_pool};
  return allocator;
}
static void run_allocation_tests(void) {
  demo_endpoint_t *endpoint = NULL, *other = NULL;
  demo_endpoint_config_t config;
  wl_allocator_t allocator = endpoint_allocator();
  CHECK(wl_fixed_pool_init(&endpoint_pool, endpoint_memory, sizeof(endpoint_memory),
      sizeof(demo_endpoint_t), DEMO_ENDPOINT_ALIGNMENT, 1) == WL_OK);
  CHECK(demo_endpoint_config_defaults(&config, test_environment_id(71, (wl_clock_t){0})) == WL_OK);
  config.environment.clock = (wl_clock_t){read_clock, NULL};
  CHECK(demo_endpoint_create(NULL, &config, &allocator) == WL_ERR_INVALID_ARG);
  CHECK(demo_endpoint_create(&endpoint, &config, NULL) == WL_ERR_INVALID_ARG);
  CHECK(allocations == 0 && endpoint == NULL);
  fail_allocation = true;
  CHECK(demo_endpoint_create(&endpoint, &config, &allocator) == WL_ERR_NO_MEM);
  CHECK(endpoint == NULL && deallocations == 0);
  fail_allocation = false;
  misalign_allocation = true;
  CHECK(demo_endpoint_create(&endpoint, &config, &allocator) == WL_ERR_INVALID_ARG);
  CHECK(endpoint == NULL && deallocations == 1 && wl_fixed_pool_in_use(&endpoint_pool) == 0);
  misalign_allocation = false;
  /* Every internal init stage still rolls back the single object allocation. */
  for (unsigned stage = 0; stage < 4; ++stage) {
    demo_endpoint_config_t bad = config;
    if (stage == 0) bad.environment.clock.now_ms = NULL;
    if (stage == 1) bad.event_budget = 0;
    if (stage == 2) bad.advanced.rpc_client_slot_count = 0;
    if (stage == 3) bad.environment.session.next = NULL;
    CHECK(demo_endpoint_create(&endpoint, &bad, &allocator) != WL_OK);
    CHECK(endpoint == NULL && wl_fixed_pool_in_use(&endpoint_pool) == 0);
  }
  CHECK(demo_endpoint_create(&endpoint, &config, &allocator) == WL_OK);
  CHECK(demo_endpoint_create(&endpoint, &config, &allocator) == WL_ERR_INVALID_ARG);
  CHECK(demo_endpoint_create(&other, &config, &allocator) == WL_ERR_NO_MEM && other == NULL);
  memset(&allocator, 0, sizeof(allocator)); /* Descriptor was copied. */
  CHECK(demo_endpoint_close(endpoint) == WL_OK);
  CHECK(demo_endpoint_init_config(endpoint, &config) == WL_OK); /* Retains allocation ownership. */
  CHECK(demo_endpoint_destroy(&endpoint) == WL_OK && endpoint == NULL);
  CHECK(demo_endpoint_destroy(&endpoint) == WL_OK);
  CHECK(wl_fixed_pool_in_use(&endpoint_pool) == 0);
  other = &client; /* Existing static endpoint is not dynamically owned. */
  CHECK(demo_endpoint_destroy(&other) == WL_ERR_INVALID_STATE && other == &client);
  allocations = deallocations = 0;
}
