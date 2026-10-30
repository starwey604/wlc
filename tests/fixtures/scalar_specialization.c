/* SPDX-License-Identifier: Apache-2.0 */
#include <assert.h>
#include "sample.c"

static uint32_t rng_state = UINT32_C(0x739a821d);
static uint32_t random32(void) {
  rng_state ^= rng_state << 13;
  rng_state ^= rng_state >> 17;
  rng_state ^= rng_state << 5;
  return rng_state;
}

#define CHECK(name, wire, n) do { \
  name##_t direct, generic; \
  memset(&direct, 0xa5, sizeof(direct)); \
  memset(&generic, 0xa5, sizeof(generic)); \
  wl_codec_status_t a = name##_decode(wire, n, &direct); \
  wl_codec_status_t b = wlc_decode(&name##_desc, wire, n, &generic); \
  assert(a == b && memcmp(&direct, &generic, sizeof(direct)) == 0); \
  if (a == WL_CODEC_OK) { \
    uint8_t actual[128], expected[128]; \
    size_t length, want_length; \
    assert(name##_encode(&direct, actual, sizeof(actual), &length) == WL_CODEC_OK); \
    assert(wlc_encode(&name##_desc, &generic, expected, sizeof(expected), &want_length) == WL_CODEC_OK); \
    assert(length == want_length && memcmp(actual, expected, length) == 0); \
    assert(name##_encoded_size(&direct) == length); \
    uint64_t hash = UINT64_C(0x71344ebd1891); size_t hashed_length; \
    wlc_hash_state_t reference = {hash, 0U, WL_CODEC_OK}; \
    assert(wlc_hash_emit_fields(&name##_desc, &generic, &reference) == WL_CODEC_OK); \
    assert(name##_wlc_detail_fingerprint(&direct, &hash, &hashed_length) == WL_CODEC_OK); \
    assert(hash == reference.hash && hashed_length == reference.length); \
    name##_value_t owned, unchanged; \
    memset(&owned, 0xa5, sizeof(owned)); unchanged = owned; \
    assert(name##_value_decode(wire, n, &owned) == WL_CODEC_OK); \
    const uint8_t malformed[] = {0}; \
    assert(name##_value_decode(malformed, sizeof(malformed), &owned) == WL_CODEC_ERR_MALFORMED); \
    assert(name##_value_decode(wire, n, &unchanged) == WL_CODEC_OK); \
    assert(memcmp(&owned, &unchanged, sizeof(owned)) == 0); \
  } \
} while (0)

static void test_wire(void) {
  /* Reversed fields, unknown legal types, noncanonical/overwide keys and
   * values, truncation, repeated required/optional fields, float bit patterns. */
  const uint8_t fixed[] = {
    0xfd,0xff,0x1f, 0x7f,0xf8,0,0,0,0,0,1,
    0x85,0x80,1, 0x7f,0xc1,0x23,0x45,
    0x81,1, 0x80,0,0,0,0,0,0,1,
    0x0d, 0x80,0,0,1,
    0x10,0x80,1, 0x19,0,0,0,0,0,0,0,0, 0x22,2,0,0xff, 0x2d,0,0,0,0
  };
  const uint8_t var[] = {
    0xf8,0xff,0x1f, 0xff,0xff,0xff,0xff,0x0f,
    0x80,0x80,1, 0x80,0x80,0x80,0x80,1,
    0x80,1, 0x80,0x80,1, 8,0
  };
  const uint8_t mixed[] = {
    0x81,0x80,1, 0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    8,0x7f, 0x85,1, 0x80,0,0,1, 0xfd,0xff,0x1f, 0x80,0,0,0
  };
  uint8_t bytes[256];
  for (size_t n = 0; n <= sizeof(fixed); ++n) CHECK(fixed, fixed, n);
  for (size_t n = 0; n <= sizeof(var); ++n) CHECK(var, var, n);
  for (size_t n = 0; n <= sizeof(mixed); ++n) CHECK(mixed, mixed, n);
  for (unsigned r = 0; r != 16384; ++r) {
    size_t n;
    if (r % 4 == 0) {
      memcpy(bytes, fixed, sizeof(fixed)); n = sizeof(fixed);
    } else if (r % 4 == 1) {
      memcpy(bytes, var, sizeof(var)); n = sizeof(var);
    } else if (r % 4 == 2) {
      memcpy(bytes, mixed, sizeof(mixed)); n = sizeof(mixed);
    } else {
      n = random32() % 128;
      for (size_t i = 0; i < n; ++i) bytes[i] = (uint8_t)random32();
    }
    if (r % 3 == 0) { memcpy(bytes + n, bytes, n); n *= 2; }
    else for (unsigned i = 0; i != 3 && n != 0; ++i) bytes[random32() % n] ^= (uint8_t)random32();
    CHECK(fixed, bytes, n); CHECK(var, bytes, n); CHECK(mixed, bytes, n);
    CHECK(var_twin, bytes, n); CHECK(prefix, bytes, n);
    CHECK(wlc_foo, bytes, n); CHECK(request_extra, bytes, n);
  }
  /* Five-byte uint32 boundary and overwide error precedence. */
  const uint64_t values[] = {0,127,128,16383,16384,UINT32_MAX,UINT64_C(0x100000000),UINT64_MAX};
  for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
    uint8_t *p = bytes; *p++ = 8;
    wlc_putv(&p, values[i]);
    for (size_t n = 0; n <= (size_t)(p - bytes); ++n) CHECK(var, bytes, n);
  }
  for (uint64_t number = 65536; number != 65539; ++number) {
    uint8_t *p = bytes;
    wlc_putv(&p, number << 3); *p++ = 0;
    CHECK(fixed, bytes, (size_t)(p - bytes));
    CHECK(var, bytes, (size_t)(p - bytes));
  }
  /* Canonical 64-bit keys can still be overwide for schema field numbers;
   * malformed 64-bit varints keep their original error before range checks. */
  const uint64_t keys[] = {UINT64_C(0x7ffff),UINT64_C(0x80000),UINT32_MAX,
    UINT64_C(0x100000008),UINT64_MAX};
  for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); ++i) {
    uint8_t *p = bytes; wlc_putv(&p, keys[i]); *p++ = 0;
    for (size_t n = 0; n <= (size_t)(p - bytes); ++n) {
      CHECK(fixed, bytes, n); CHECK(var, bytes, n);
    }
  }
  const uint8_t noncanonical[][7] = {
    {8,128,0}, {8,255,0}, {8,128,128,0}, {8,128,128,128,0},
    {8,128,128,128,128,0}, {8,128,128,128,128,128,0}
  };
  for (size_t i = 0; i < sizeof(noncanonical) / sizeof(noncanonical[0]); ++i)
    CHECK(var, noncanonical[i], sizeof(noncanonical[i]));
}

int main(void) {
  assert(sizeof(var_t) == sizeof(var_twin_t));
  assert(offsetof(var_t, a) == offsetof(var_twin_t, id));
  assert(offsetof(var_t, has_a) == offsetof(var_twin_t, has_id));
  assert(offsetof(var_t, b) == offsetof(var_twin_t, limit));
  assert(offsetof(var_t, has_b) == offsetof(var_twin_t, has_limit));
  assert(offsetof(var_t, c) == offsetof(var_twin_t, left));
  assert(offsetof(var_t, has_c) == offsetof(var_twin_t, has_left));
  assert(offsetof(var_t, d) == offsetof(var_twin_t, right));
  assert(offsetof(var_t, has_d) == offsetof(var_twin_t, has_right));
  const uint8_t member_names[] = {13, 0x12, 0x34, 0x56, 0x78, 21, 0, 0, 0, 1};
  CHECK(wlc_foo, member_names, 5);
  CHECK(request_extra, member_names, sizeof(member_names));
  test_wire();
  return 0;
}
