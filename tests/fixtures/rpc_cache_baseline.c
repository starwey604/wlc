/* SPDX-License-Identifier: Apache-2.0 */
#include "wirelink/rpc.h"
#include <stdio.h>
#include <string.h>

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)

static int run(uint16_t count, uint16_t capacity, wl_rpc_cache_policy_t policy) {
  wl_rpc_server_t server;
  wl_rpc_server_pending_slot_t pending[8];
  wl_rpc_server_cache_slot_t cache[8];
  uint8_t bytes[8U * 2048U];
  wl_rpc_server_request_t requests[8], request;
  wl_rpc_server_response_t response;
  wl_rpc_server_disposition_t disposition;
  wl_rpc_request_identity_t identity = {1U, 2U, 3U, 42U, 1U};
  const wl_rpc_server_config_t config = {
      pending, count, cache, count, bytes, sizeof(bytes), capacity,
      1000U, 10000U, policy};
  CHECK(wl_rpc_server_init(&server, &config) == WL_RPC_OK);
  for (uint16_t i = 0U; i < count; ++i) {
    identity.operation_id = i + 1U;
    CHECK(wl_rpc_server_begin(&server, &identity, 100U, &disposition,
        &requests[i], &response) == WL_RPC_OK);
    CHECK(disposition == WL_RPC_SERVER_NEW);
    CHECK(wl_rpc_server_complete(&server, &requests[i], 0, NULL, 0U,
        100U, &response) == WL_RPC_OK);
  }
  identity.operation_id = count + 1U;
  /* READY is protected even with eviction, not just a pending reservation. */
  CHECK(wl_rpc_server_begin(&server, &identity, 100U, &disposition,
      &request, &response) == WL_RPC_ERR_CACHE_FULL);
  for (uint16_t i = 0U; i < count; ++i) {
    CHECK(wl_rpc_server_response_acquire(&server, &response) == WL_RPC_OK);
    CHECK(wl_rpc_server_response_sent(&server, &response) == WL_RPC_OK);
  }
  unsigned accepted = count;
  for (unsigned i = count; i < 100U; ++i) {
    identity.operation_id = i + 1U;
    wl_rpc_err_t result = wl_rpc_server_begin(&server, &identity, 100U,
        &disposition, &request, &response);
    if (policy == WL_RPC_CACHE_REJECT_NEW) {
      CHECK(result == WL_RPC_ERR_CACHE_FULL);
      continue;
    }
    CHECK(result == WL_RPC_OK && disposition == WL_RPC_SERVER_NEW);
    ++accepted;
    CHECK(wl_rpc_server_complete(&server, &request, 0, NULL, 0U,
        100U, &response) == WL_RPC_OK);
    CHECK(wl_rpc_server_response_acquire(&server, &response) == WL_RPC_OK);
    CHECK(wl_rpc_server_response_sent(&server, &response) == WL_RPC_OK);
  }
  CHECK(accepted == (policy == WL_RPC_CACHE_REJECT_NEW ? count : 100U));
  printf("cache policy=%s slots=%u response_capacity=%u pool_bytes=%zu accepted=%u/100 before TTL\n",
      policy == WL_RPC_CACHE_REJECT_NEW ? "strict" : "recent", count, capacity,
      (sizeof(pending[0]) + sizeof(cache[0]) + capacity) * count, accepted);
  return 0;
}

int main(void) {
  const uint16_t counts[] = {1U, 4U, 8U};
  const uint16_t capacities[] = {18U, 63U, 2046U};
  for (unsigned i = 0U; i < 3U; ++i)
    for (unsigned j = 0U; j < 3U; ++j) {
      CHECK(run(counts[i], capacities[j], WL_RPC_CACHE_REJECT_NEW) == 0);
      CHECK(run(counts[i], capacities[j], WL_RPC_CACHE_EVICT_OLDEST) == 0);
    }
  return 0;
}
