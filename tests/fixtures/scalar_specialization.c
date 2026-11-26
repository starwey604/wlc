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
  name##_t direct, canary, roundtrip; \
  memset(&direct, 0xa5, sizeof(direct)); \
  canary = direct; \
  wl_codec_status_t a = name##_decode(wire, n, &direct); \
  if (a != WL_CODEC_OK) { \
    assert(memcmp(&direct, &canary, sizeof(direct)) == 0); \
  } else { \
    uint8_t actual[128], reencoded[128]; \
    size_t length = 0, reencoded_length = 0; \
    assert(name##_encode(&direct, actual, sizeof(actual), &length) == WL_CODEC_OK); \
    assert(name##_encoded_size(&direct) == length); \
    for (size_t cap = 0; cap < length; ++cap) { \
      size_t small_len = 9999; \
      assert(name##_encode(&direct, actual, cap, &small_len) == WL_CODEC_ERR_CAPACITY); \
    } \
    assert(name##_decode(actual, length, &roundtrip) == WL_CODEC_OK); \
    assert(name##_encode(&roundtrip, reencoded, sizeof(reencoded), &reencoded_length) == WL_CODEC_OK); \
    assert(length == reencoded_length && memcmp(actual, reencoded, length) == 0); \
    name##_t saved = direct; \
    const uint8_t malformed[] = {0}; \
    assert(name##_decode(malformed, sizeof(malformed), &direct) == WL_CODEC_ERR_MALFORMED); \
    assert(memcmp(&direct, &saved, sizeof(direct)) == 0); \
  } \
} while (0)

static void test_ordered_and_fallback(void) {
  /* Prefix has two required varints and an optional third one. */
  static const uint8_t ordered[] = {8, 1, 16, 2, 24, 3};
  static const uint8_t reordered[] = {16, 2, 8, 1, 24, 3};
  static const uint8_t unknown[] = {8, 1, 34, 1, 99, 16, 2, 24, 3, 40, 0};
  prefix_t out;
  const uint8_t *valid[] = {ordered, reordered, unknown};
  size_t sizes[] = {sizeof(ordered), sizeof(reordered), sizeof(unknown)};
  for (unsigned i = 0; i < 3; ++i) {
    assert(prefix_decode(valid[i], sizes[i], &out) == WL_CODEC_OK);
    assert(out.a == 1 && out.abc == 2 && out.has_ab && out.ab == 3);
  }
  assert(prefix_decode(ordered, 4, &out) == WL_CODEC_OK && !out.has_ab);

  static const struct {
    uint8_t wire[10]; size_t length; wl_codec_status_t status;
  } errors[] = {
    {{8,1}, 2, WL_CODEC_ERR_MISSING_REQUIRED_FIELD},
    {{16,2,8,1,16,2}, 6, WL_CODEC_ERR_DUPLICATE_FIELD},
    {{8,1,21,0,0,0,0}, 7, WL_CODEC_ERR_WIRE_TYPE},
    {{8,1,16,128}, 4, WL_CODEC_ERR_MALFORMED},
    {{8,1,16,128,0}, 5, WL_CODEC_ERR_MALFORMED},
    {{8,1,16,128,128,128,128,16}, 8, WL_CODEC_ERR_OVERFLOW},
    {{8,1,16,2,34,4,1}, 7, WL_CODEC_ERR_MALFORMED},
  };
  unsigned char saved[sizeof(out)];
  memcpy(saved, &out, sizeof(out));
  for (size_t i = 0; i < sizeof(errors)/sizeof(errors[0]); ++i) {
    assert(prefix_decode(errors[i].wire, errors[i].length, &out) == errors[i].status);
    assert(memcmp(&out, saved, sizeof(out)) == 0);
  }
}

static void test_wire(void) {
  /* Reversed fields, unknown legal types, noncanonical/overwide keys and
   * values, truncation, repeated required/optional fields, float bit patterns (LE). */
  const uint8_t fixed[] = {
    0xf9,0xff,0x1f, 0x01,0x00,0x00,0x00,0x00,0x00,0xf8,0x7f,
    0x85,0x80,0x01, 0x45,0x23,0xc1,0x7f,
    0x81,0x01, 0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x80,
    0x0d, 0x01,0x00,0x00,0x80,
    0x10,0x80,0x01, 0x19,0,0,0,0,0,0,0,0, 0x22,2,0,0xff, 0x2d,0,0,0,0
  };
  const uint8_t var[] = {
    0xf8,0xff,0x1f, 0xff,0xff,0xff,0xff,0x0f,
    0x80,0x80,0x01, 0x80,0x80,0x80,0x80,0x01,
    0x80,0x01, 0x80,0x80,0x01, 8,0
  };
  const uint8_t mixed[] = {
    0x81,0x80,0x01, 0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    8,0x7f, 0x85,0x01, 0x01,0x00,0x00,0x80, 0xfd,0xff,0x1f, 0x00,0x00,0x00,0x80
  };
  uint8_t bytes[256];
  fixed_t fdec;
  assert(fixed_decode(fixed, sizeof(fixed), &fdec) == WL_CODEC_OK);
  assert(fdec.a == UINT32_C(0x80000001));
  assert(fdec.has_b && fdec.b == UINT64_C(0x8000000000000001));
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
    p = wl_codec_putv(p, values[i]);
    for (size_t n = 0; n <= (size_t)(p - bytes); ++n) CHECK(var, bytes, n);
  }
  for (uint64_t number = 65536; number != 65539; ++number) {
    uint8_t *p = bytes;
    p = wl_codec_putv(p, number << 3); *p++ = 0;
    CHECK(fixed, bytes, (size_t)(p - bytes));
    CHECK(var, bytes, (size_t)(p - bytes));
  }
  /* Canonical 64-bit keys can still be overwide for schema field numbers;
   * malformed 64-bit varints keep their original error before range checks. */
  const uint64_t keys[] = {UINT64_C(0x7ffff),UINT64_C(0x80000),UINT32_MAX,
    UINT64_C(0x100000008),UINT64_MAX};
  for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); ++i) {
    uint8_t *p = bytes; p = wl_codec_putv(p, keys[i]); *p++ = 0;
    for (size_t n = 0; n <= (size_t)(p - bytes); ++n) {
      CHECK(fixed, bytes, n); CHECK(var, bytes, n);
    }
  }
  const uint8_t noncanonical[][7] = {
    {8,128,0}, {8,255,0}, {8,128,128,0}, {8,128,128,128,0},
    {8,128,128,128,128,0}, {8,128,128,128,128,128,0}
  };
  for (size_t i = 0; i < sizeof(noncanonical) / sizeof(noncanonical[0]); ++i) {
    var_t dummy;
    assert(var_decode(noncanonical[i], sizeof(noncanonical[i]), &dummy) == WL_CODEC_ERR_MALFORMED);
  }
}

int main(void) {
  assert(sizeof(var_t) == sizeof(var_twin_t));
  assert(offsetof(var_t, a) == offsetof(var_twin_t, id));
  assert(offsetof(var_t, b) == offsetof(var_twin_t, limit));
  assert(offsetof(var_t, has_b) == offsetof(var_twin_t, has_limit));
  assert(offsetof(var_t, c) == offsetof(var_twin_t, left));
  assert(offsetof(var_t, d) == offsetof(var_twin_t, right));
  assert(offsetof(var_t, has_d) == offsetof(var_twin_t, has_right));
  const uint8_t member_names[] = {13, 0x78, 0x56, 0x34, 0x12, 21, 1, 0, 0, 0};
  CHECK(wlc_foo, member_names, 5);
  CHECK(request_extra, member_names, sizeof(member_names));
  test_ordered_and_fallback();
  test_wire();
  return 0;
}
