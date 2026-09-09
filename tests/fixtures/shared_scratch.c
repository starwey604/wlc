/* SPDX-License-Identifier: Apache-2.0 */
/* Include the generated implementation to compare its private fingerprint
 * against the original algorithm, independent of generation-time constants. */
#include "demo_runtime.c"
#include <assert.h>
#include <stddef.h>
#include "wirelink/loopback.h"

_Static_assert(offsetof(demo_runtime_instance_t, small_scratch) ==
               offsetof(demo_runtime_instance_t, large_scratch), "decode scratch must alias");

static uint64_t reference(const uint8_t *data, size_t length) {
  static const uint8_t domain[] = "wlc.rpc.canonical-request.v1";
  uint64_t hash = UINT64_C(0xcbf29ce484222325);
  for (size_t i = 0; i + 1 < sizeof(domain); ++i) {
    hash ^= domain[i];
    hash *= UINT64_C(0x100000001b3);
  }
  hash = (hash ^ UINT64_C(0xff)) * UINT64_C(0x100000001b3);
  for (size_t i = 0; i < length; ++i) {
    hash ^= data[i];
    hash *= UINT64_C(0x100000001b3);
  }
  return hash;
}

static demo_endpoint_t client_endpoint, server_endpoint;
static wl_loopback_t cable;
static demo_small_request_token_t delayed;
static small_request_value_t saved_input;
static unsigned completed;
static wl_time_ms_t now(void *context) { (void)context; return 100U; }
static wl_err_t next_identity(void *context, uint64_t *out) {
  *out = ++*(uint64_t *)context;
  return WL_OK;
}
static int32_t defer_small(void *context, const small_request_t *input,
    const demo_small_request_token_t *token, wl_delivery_t delivery) {
  (void)context; (void)delivery;
  delayed = *token;
  assert(small_request_value_from_view(input, &saved_input) == WL_CODEC_OK);
  return 0;
}
static int32_t large(void *context, const large_request_value_t *input,
    large_response_value_t *output) {
  (void)context;
  assert(input->values[31] == 42.0);
  output->has_data = true;
  output->data.length = 512;
  memset(output->data.data, 0xa5, 512);
  return 0;
}
static void small_done(void *context, const wl_rpc_completion_t *result,
    const small_response_value_t *output) {
  (void)context;
  assert(result->status == WL_RPC_SUCCESS && output != NULL);
  ++completed;
}
static void large_done(void *context, const wl_rpc_completion_t *result,
    const large_response_value_t *output) {
  (void)context;
  assert(result->status == WL_RPC_SUCCESS && output != NULL);
  assert(output->data.length == 512 && output->data.data[511] == 0xa5);
  ++completed;
}
static void progress(void) {
  assert(demo_endpoint_step(&client_endpoint) == WL_OK);
  assert(demo_endpoint_step(&server_endpoint) == WL_OK);
}
static void deferred_work_survives_another_service(void) {
  uint64_t identity = 1;
  const wl_environment_t environment = {{now, NULL}, {next_identity, &identity}};
  demo_endpoint_config_t config;
  assert(demo_endpoint_init(&client_endpoint, environment) == WL_OK);
  assert(demo_endpoint_config_defaults(&config, environment) == WL_OK);
  config.advanced.small_request_handler = defer_small;
  config.on_large = large;
  assert(demo_endpoint_init_config(&server_endpoint, &config) == WL_OK);
  assert(wl_loopback_connect(&cable, demo_endpoint_handle(&client_endpoint),
      demo_endpoint_handle(&server_endpoint)) == WL_OK);
  small_request_value_t small_input;
  small_request_value_clear(&small_input);
  small_input.has_name = true;
  small_input.name.length = 4;
  memcpy(small_input.name.data, "kept", 4);
  assert(demo_endpoint_small_async(&client_endpoint, &small_input, 1000, small_done, NULL, NULL) == WL_OK);
  for (unsigned i = 0; i < 4; ++i) progress();
  assert(completed == 0 && saved_input.name.length == 4);
  large_request_value_t large_input;
  large_request_value_clear(&large_input);
  large_input.has_values = true;
  large_input.values[31] = 42.0;
  assert(demo_endpoint_large_async(&client_endpoint, &large_input, 1000, large_done, NULL, NULL) == WL_OK);
  for (unsigned i = 0; i < 64 && completed != 1; ++i) progress();
  assert(completed == 1);
  assert(saved_input.name.length == 4 && memcmp(saved_input.name.data, "kept", 4) == 0);
  small_response_t small_output;
  small_response_clear(&small_output);
  const demo_runtime_result_t result = demo_small_server_complete(
      demo_endpoint_runtime(&server_endpoint), &delayed, &small_output, 100);
  assert(demo_runtime_result_ok(&result));
  for (unsigned i = 0; i < 64 && completed != 2; ++i) progress();
  assert(completed == 2);
  for (unsigned i = 0; i < 4; ++i) progress();
  assert(demo_endpoint_close(&client_endpoint) == WL_OK);
  assert(demo_endpoint_close(&server_endpoint) == WL_OK);
}

int main(void) {
  demo_runtime_config_t config;
  demo_runtime_requirements_t initial, grown, client;
  demo_runtime_default_storage_t storage;
  demo_runtime_instance_t instance;
  uint8_t payload[257];
  assert(demo_runtime_config_defaults(&config) == WL_OK);
  assert(demo_runtime_config_enable_server(&config) == WL_OK);
  assert(demo_runtime_requirements(&config, &initial) == WL_OK);
  config.rpc_server_response_capacity += 100;
  assert(demo_runtime_requirements(&config, &grown) == WL_OK);
  assert(grown.storage_size == initial.storage_size + 100);
  config.rpc_server_response_capacity -= 100;
  const demo_runtime_storage_t descriptor = demo_runtime_default_storage_descriptor(&storage);
  assert(demo_runtime_init(&instance, &config, &descriptor) == WL_OK);
  assert((void *)instance.runtime.small.request_scratch == (void *)instance.runtime.large.request_scratch);
  assert(instance.runtime.large.response_scratch == NULL); /* Client role disabled. */
  assert((uintptr_t)instance.runtime.large.request_scratch % _Alignof(large_request_t) == 0);
  config.rpc_server_cache_ttl_ms = UINT32_MAX;
  assert(demo_runtime_requirements(&config, &grown) == WL_ERR_INVALID_ARG);
  config.rpc_server_enabled = 0;
  assert(demo_runtime_config_enable_client(&config) == WL_OK);
  assert(demo_runtime_requirements(&config, &client) == WL_OK); /* no server scratch */
  assert(demo_runtime_requirements(&config, &grown) == WL_OK);
  assert(client.storage_size == grown.storage_size);
  for (size_t length = 0; length <= 17; ++length) {
    small_request_t request;
    size_t canonical_length, hashed_length;
    uint64_t hash = reference(NULL, 0);
    small_request_clear(&request);
    request.has_name = true;
    request.name = (wl_codec_string_t){"abcdefghijklmnopq", length};
    assert(small_request_encode(&request, payload, sizeof(payload), &canonical_length) == WL_CODEC_OK);
    /* Exercise the private seam only after decode, as the runtime does. */
    assert(small_request_decode(payload, canonical_length, &request) == WL_CODEC_OK);
    assert(small_request_wlc_detail_fingerprint(&request, &hash, &hashed_length) == WL_CODEC_OK);
    assert(hashed_length == canonical_length && hash == reference(payload, canonical_length));
  }
  deferred_work_survives_another_service();
  return 0;
}
