/* SPDX-License-Identifier: Apache-2.0 */
#include <assert.h>
#include <stdio.h>
#include <string.h>
static size_t utf8_calls, fingerprint_calls, zero_bytes;
static void *count_zero(void *out, int byte, size_t length) {
  zero_bytes += length;
  return memset(out, byte, length);
}
#define memset count_zero
#include "validation.c"
#undef memset
#include "validation_runtime.c"

static uint64_t reference(const uint8_t *data, size_t length) {
  const uint8_t domain[] = "wlc.rpc.canonical-request.v1";
  uint64_t hash = UINT64_C(0xcbf29ce484222325);
  for (size_t i = 0; i + 1 < sizeof(domain); ++i) hash = (hash ^ domain[i]) * UINT64_C(0x100000001b3);
  hash = (hash ^ 255U) * UINT64_C(0x100000001b3);
  for (size_t i = 0; i < length; ++i) hash = (hash ^ data[i]) * UINT64_C(0x100000001b3);
  return hash;
}
static size_t reverse_fields(const uint8_t *in, size_t length, uint8_t *out) {
  size_t starts[32], ends[32], count = 0, at = 0, used = 0;
  while (at < length) {
    uint64_t key;
    starts[count] = at;
    assert(wlc_getv(in, length, &at, &key) == WL_CODEC_OK);
    assert(wlc_skip((uint8_t)(key & 7U), in, length, &at) == WL_CODEC_OK);
    ends[count++] = at;
  }
  while (count) {
    --count;
    size_t n = ends[count] - starts[count];
    memcpy(out + used, in + starts[count], n);
    used += n;
  }
  return used;
}
static void check_request(const uint8_t *wire, size_t length, uint64_t expected) {
  request_t view;
  request_value_t owned, public_value;
  uint8_t canonical[REQUEST_MAX_ENCODED_SIZE];
  size_t n, hashed_length;
  uint64_t hash = reference(NULL, 0);
  assert(request_decode(wire, length, &view) == WL_CODEC_OK);
  utf8_calls = 0;
  assert(request_wlc_detail_fingerprint(&view, &hash, &hashed_length) == WL_CODEC_OK);
  assert(utf8_calls == 0);
  assert(request_encode(&view, canonical, sizeof(canonical), &n) == WL_CODEC_OK);
  assert(hash == reference(canonical, n) && n == hashed_length && hash == expected);
  memset(&owned, 0xA5, sizeof(owned));
  utf8_calls = zero_bytes = 0;
  request_wlc_detail_value_copy(&view, &owned);
  assert(utf8_calls == 0 && zero_bytes == sizeof(owned));
  assert(request_value_from_view(&view, &public_value) == WL_CODEC_OK);
  assert(memcmp(&owned, &public_value, sizeof(owned)) == 0);
  assert(owned.child.text.data[owned.child.text.length] == 0);
  assert(owned.child.greeting.length == 3 && !owned.child.has_greeting);
  assert(!owned.has_default_unsigned && owned.default_unsigned == UINT32_MAX);
  assert(!owned.has_default_signed && owned.default_signed == INT32_MIN);
  assert(!owned.has_default_wide && owned.default_wide == INT64_MIN);
  for (size_t i = owned.body.length; i < sizeof(owned.body.data); ++i) assert(owned.body.data[i] == 0);
  /* Public conversion still validates, and failure leaves even padding intact. */
  request_value_t before = owned;
  view.child.text = (wl_codec_string_t){"\xC0\x80", 2};
  assert(request_value_from_view(&view, &owned) == WL_CODEC_ERR_UTF8);
  assert(memcmp(&before, &owned, sizeof(owned)) == 0);
  view.child.text = (wl_codec_string_t){"a", 65};
  assert(request_value_from_view(&view, &owned) == WL_CODEC_ERR_INVALID_VALUE);
  assert(memcmp(&before, &owned, sizeof(owned)) == 0);
}
static uint32_t random_state = UINT32_C(0x29ab701f);
static uint32_t next_random(void) {
  random_state ^= random_state << 13;
  random_state ^= random_state >> 17;
  random_state ^= random_state << 5;
  return random_state;
}
static void property_cases(request_t input) {
  uint8_t wire[REQUEST_MAX_ENCODED_SIZE + 32], shuffled[sizeof(wire)], canonical[sizeof(wire)];
  char text[64]; uint8_t body[128];
  for (unsigned round = 0; round < 512; ++round) {
    size_t n = next_random() % 65U, b = next_random() % 129U;
    for (size_t i = 0; i < n; ++i) text[i] = (char)(' ' + next_random() % 95U);
    for (size_t i = 0; i < b; ++i) body[i] = (uint8_t)next_random();
    input.child.text = (wl_codec_string_t){text, n};
    input.body = (wl_codec_bytes_t){body, b};
    input.has_samples = (next_random() & 1U) != 0;
    input.child.has_weights = (next_random() & 1U) != 0;
    input.has_integer = (next_random() & 1U) != 0;
    input.integer = (int64_t)next_random() - (int64_t)next_random();
    uint32_t bits = next_random(); memcpy(&input.single, &bits, sizeof(bits));
    uint64_t wide = ((uint64_t)next_random() << 32) | next_random();
    memcpy(&input.double_value, &wide, sizeof(wide));
    for (size_t i = 0; i < 16; ++i) input.samples[i] = next_random();
    size_t length, encoded, hashed;
    assert(request_encode(&input, wire, sizeof(wire), &length) == WL_CODEC_OK);
    assert(reverse_fields(wire, length, shuffled) == length);
    request_t decoded;
    assert(request_decode(shuffled, length, &decoded) == WL_CODEC_OK);
    assert(request_encode(&decoded, canonical, sizeof(canonical), &encoded) == WL_CODEC_OK);
    uint64_t hash = reference(NULL, 0);
    assert(request_wlc_detail_fingerprint(&decoded, &hash, &hashed) == WL_CODEC_OK);
    assert(hashed == encoded && hash == reference(canonical, encoded));
    /* The codec sink has no RPC policy: caller-supplied seeds are preserved. */
    uint64_t seed = wide, expected = seed;
    for (size_t i = 0; i < encoded; ++i) expected = (expected ^ canonical[i]) * UINT64_C(0x100000001b3);
    assert(request_wlc_detail_fingerprint(&decoded, &seed, &hashed) == WL_CODEC_OK && seed == expected);
    request_value_t owned, checked;
    request_wlc_detail_value_copy(&decoded, &owned);
    assert(request_value_from_view(&decoded, &checked) == WL_CODEC_OK);
    assert(memcmp(&owned, &checked, sizeof(owned)) == 0);
  }
}
static wl_time_ms_t now(void *context) { (void)context; return 1; }
static wl_err_t identity(void *context, uint64_t *out) { (void)context; *out = 99; return WL_OK; }
static unsigned calls;
static int32_t execute(void *context, const request_value_t *in, reply_value_t *out) {
  (void)context;
  assert(in->child.text.length == 5 && memcmp(in->child.text.data, "ABCDE", 5) == 0);
  ++calls;
  out->has_result = true; out->result = 42;
  return 0;
}
static void runtime_cases(const uint8_t *body, size_t length, const uint8_t *reordered) {
  static validation_endpoint_t server;
  validation_endpoint_config_t config;
  wl_environment_t environment = {{now, NULL}, {identity, NULL}};
  assert(validation_endpoint_config_defaults(&config, environment) == WL_OK);
  config.on_execute = execute;
  assert(validation_endpoint_init_config(&server, &config) == WL_OK);
  uint8_t wire[REQUEST_MAX_ENCODED_SIZE + 32];
  validation_rpc_header_write(wire, 1, 7, 0, 42);
  memcpy(wire + 20, body, length);
  wl_event_t event = {.type = WL_EVT_UNRELIABLE_RX, .message_id = REQUEST_MESSAGE_ID,
      .payload = wire, .payload_len = length + 20};
  wl_ctx_t *link = wl_endpoint_link(validation_endpoint_handle(&server));
  validation_runtime_t *runtime = validation_endpoint_runtime(&server);
  utf8_calls = fingerprint_calls = 0;
  validation_runtime_result_t r = validation_runtime_dispatch_event(link, &event, runtime, 1);
  assert(r.domain == VALIDATION_RUNTIME_OK && calls == 1);
  assert(utf8_calls == 1 && fingerprint_calls == 1);
  memcpy(wire + 20, reordered, length);
  r = validation_runtime_dispatch_event(link, &event, runtime, 1);
  assert(r.domain == VALIDATION_RUNTIME_OK && r.detail.rpc.rpc_disposition == WL_RPC_SERVER_REPLAY && calls == 1);
  const uint8_t unknown[] = {0xF8, 0x07, 1};
  memcpy(wire + 20 + length, unknown, sizeof(unknown)); event.payload_len += sizeof(unknown);
  r = validation_runtime_dispatch_event(link, &event, runtime, 1);
  assert(r.domain == VALIDATION_RUNTIME_OK && calls == 1);
  memcpy(wire + 20, body, length); event.payload_len = length + 20;
  size_t text = 0;
  for (size_t i = 20; i + 5 <= event.payload_len; ++i)
    if (memcmp(wire + i, "ABCDE", 5) == 0) { text = i; break; }
  assert(text != 0); wire[text] = 'Z';
  r = validation_runtime_dispatch_event(link, &event, runtime, 1);
  assert(r.domain == VALIDATION_RUNTIME_RPC_ERROR && r.detail.rpc.rpc_result == WL_RPC_ERR_OPERATION_CONFLICT && calls == 1);
  wire[text] = 0xC0;
  size_t before = fingerprint_calls;
  r = validation_runtime_dispatch_event(link, &event, runtime, 1);
  assert(r.domain == VALIDATION_RUNTIME_CODEC_ERROR && r.detail.rpc.codec_status == WL_CODEC_ERR_UTF8);
  assert(calls == 1 && fingerprint_calls == before);
  assert(validation_endpoint_close(&server) == WL_OK);
}

int main(void) {
  request_t input;
  uint8_t wire[REQUEST_MAX_ENCODED_SIZE + 8], reordered[sizeof(wire)];
  size_t length;
  request_clear(&input);
  request_value_t defaults;
  request_value_clear(&defaults);
  assert(!defaults.has_unsigned_value && defaults.unsigned_value == UINT64_MAX);
  input.has_child = input.has_body = input.has_samples = true;
  input.child.has_text = input.child.has_weights = true;
  input.child.text = (wl_codec_string_t){"ABCDE", 5};
  input.body = (wl_codec_bytes_t){(const uint8_t *)"payload", 7};
  input.has_integer = input.has_unsigned_value = input.has_tiny = input.has_enabled = true;
  input.integer = INT64_MIN; input.unsigned_value = UINT64_MAX; input.tiny = INT8_MIN; input.enabled = true;
  input.has_single = input.has_double_value = true;
  const uint32_t nan = UINT32_C(0x7fc01234);
  memcpy(&input.single, &nan, sizeof(nan)); input.double_value = -0.0;
  for (size_t i = 0; i < 16; ++i) input.samples[i] = (uint32_t)i * 123456789U;
  for (size_t i = 0; i < 4; ++i) input.child.weights[i] = (double)i - 0.5;
  assert(request_encode(&input, wire, sizeof(wire), &length) == WL_CODEC_OK);
  const uint64_t hash = reference(wire, length);
  check_request(wire, length, hash);
  assert(reverse_fields(wire, length, reordered) == length);
  check_request(reordered, length, hash);
  runtime_cases(wire, length, reordered);
  property_cases(input);
  /* Valid truncations must still match the original encoder; invalid ones
   * must leave owned output unchanged and never require a trusted operation. */
  for (size_t n = 0; n < length; ++n) {
    request_t decoded;
    request_value_t owned, before;
    memset(&owned, 0xA5, sizeof(owned)); before = owned;
    int result = request_decode(wire, n, &decoded);
    if (result == WL_CODEC_OK) {
      uint8_t canonical[sizeof(wire)]; size_t bytes, hashed; uint64_t fingerprint = reference(NULL, 0);
      assert(request_encode(&decoded, canonical, sizeof(canonical), &bytes) == WL_CODEC_OK);
      assert(request_wlc_detail_fingerprint(&decoded, &fingerprint, &hashed) == WL_CODEC_OK);
      assert(bytes == hashed && fingerprint == reference(canonical, bytes));
    } else {
      assert(request_value_decode(wire, n, &owned) != WL_CODEC_OK);
      assert(memcmp(&before, &owned, sizeof(owned)) == 0);
    }
  }
  /* Repeated backing remains caller-configured in the advanced codec. */
  uint32_t items[] = {0, 127, 128, UINT32_MAX}, decoded_items[4];
  child_t children[2], decoded_children[2];
  children[0] = input.child; children[1] = input.child;
  repeated_t repeated = {0}, decoded = {0};
  repeated.items = items; repeated.items_count = repeated.items_capacity = 4;
  repeated.children = children; repeated.children_count = repeated.children_capacity = 2;
  decoded.items = decoded_items; decoded.items_capacity = 4;
  decoded.children = decoded_children; decoded.children_capacity = 2;
  assert(repeated_encode(&repeated, wire, sizeof(wire), &length) == WL_CODEC_OK);
  assert(repeated_decode(wire, length, &decoded) == WL_CODEC_OK);
  uint64_t actual = reference(NULL, 0); size_t hashed;
  assert(repeated_wlc_detail_fingerprint(&decoded, &actual, &hashed) == WL_CODEC_OK);
  assert(actual == reference(wire, length) && hashed == length);
  printf("validated RPC: UTF8 once, canonical identity preserved, owned zero pass once\n");
  return 0;
}
