/* SPDX-License-Identifier: Apache-2.0 */
#include "mixed_runtime.c"
#include <assert.h>

int main(void) {
  mixed_runtime_config_t config;
  assert(mixed_runtime_config_defaults(&config) == WL_OK);
#if REPLAY
  assert(config.rpc_server_response_capacity == 24U);
#else
  assert(config.rpc_server_correlate_response_capacity == 24U);
#endif
  const uint8_t positional[] = {0x00, 0x80, 0x01};
  const uint8_t tagged_parent[] = {0x0a, 0x03, 0x08, 0x80, 0x01};
  child_t child = {.has_value = true, .value = 128U}, decoded_child;
  parent_t parent = {.has_child = true, .child = child}, decoded_parent;
  uint8_t bytes[32]; size_t length;
  assert(mixed_rpc_child_encode(&child, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(positional) && memcmp(bytes, positional, length) == 0);
  assert(mixed_rpc_child_decode(positional, sizeof(positional), &decoded_child) == WL_CODEC_OK);
  assert(decoded_child.value == child.value && !decoded_child.has_extra && decoded_child.extra == 19U);
  assert(mixed_rpc_parent_encode(&parent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(tagged_parent) && memcmp(bytes, tagged_parent, length) == 0);
  assert(mixed_rpc_parent_decode(tagged_parent, sizeof(tagged_parent), &decoded_parent) == WL_CODEC_OK);
  assert(decoded_parent.child.value == child.value && !decoded_parent.child.has_extra && decoded_parent.child.extra == 19U);
  child_value_t owned_child;
  parent_value_t owned_parent;
  assert(mixed_rpc_child_value_decode(positional, sizeof(positional), &owned_child) == WL_CODEC_OK);
  assert(owned_child.value == child.value && !owned_child.has_extra && owned_child.extra == 19U);
  assert(mixed_rpc_parent_value_decode(tagged_parent, sizeof(tagged_parent), &owned_parent) == WL_CODEC_OK);
  assert(owned_parent.child.value == child.value && !owned_parent.child.has_extra && owned_parent.child.extra == 19U);
  assert(mixed_rpc_child_value_encode(&owned_child, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(positional) && memcmp(bytes, positional, length) == 0);
  assert(mixed_rpc_parent_value_encode(&owned_parent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(tagged_parent) && memcmp(bytes, tagged_parent, length) == 0);
  return 0;
}
