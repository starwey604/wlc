/* SPDX-License-Identifier: Apache-2.0 */
#include <assert.h>
#include <limits.h>
#include "required_fixed.c"

static uint32_t state = UINT32_C(0xc43162a9);
static uint32_t random32(void) {
  state ^= state << 13;
  state ^= state >> 17;
  state ^= state << 5;
  return state;
}

#define ENCODE_CHECK(name, value) do { \
  const size_t size = name##_encoded_size(&(value)); \
  for (size_t cap = 0; cap <= 40; ++cap) { \
    for (unsigned nulls = 0; nulls < 4; ++nulls) { \
      uint8_t actual[48], expected[48]; \
      memset(actual, 0xa5, sizeof(actual)); memset(expected, 0xa5, sizeof(expected)); \
      size_t length = SIZE_MAX - 7U, wanted = length; \
      wl_codec_status_t a = name##_encode(&(value), (nulls & 1U) ? NULL : actual + 1, cap, \
                                           (nulls & 2U) ? NULL : &length); \
      wl_codec_status_t b = wlc_encode(&name##_desc, &(value), (nulls & 1U) ? NULL : expected + 1, cap, \
                                       (nulls & 2U) ? NULL : &wanted); \
      assert(a == b && length == wanted && memcmp(actual, expected, sizeof(actual)) == 0); \
      if (a != WL_CODEC_OK) { \
        for (size_t i = 0; i < sizeof(actual); ++i) assert(actual[i] == 0xa5); \
        assert(length == SIZE_MAX - 7U); \
      } else { \
        assert(length == size && actual[0] == 0xa5 && actual[1 + size] == 0xa5); \
        name##_t decoded; name##_clear(&decoded); \
        assert(name##_decode(actual + 1, length, &decoded) == WL_CODEC_OK); \
        uint8_t roundtrip[48]; size_t roundtrip_length; \
        assert(name##_encode(&decoded, roundtrip, sizeof(roundtrip), &roundtrip_length) == WL_CODEC_OK); \
        assert(length == roundtrip_length && memcmp(actual + 1, roundtrip, length) == 0); \
      } \
    } \
  } \
} while (0)

static void null_values(void) {
  uint8_t bytes[48]; memset(bytes, 0xa5, sizeof(bytes)); size_t length = 99;
  assert(sample_encode(NULL, bytes, sizeof(bytes), &length) == WL_CODEC_ERR_INVALID_VALUE);
  assert(sample_encode(NULL, NULL, 0, NULL) == WL_CODEC_ERR_INVALID_VALUE);
  assert(sample_encoded_size(NULL) == SIZE_MAX && length == 99);
  for (size_t i = 0; i < sizeof(bytes); ++i) assert(bytes[i] == 0xa5);
}

int main(void) {
  null_values();
  const uint8_t golden[] = {13,1,2,3,4,21,0,0,0,0,29,0xff,0xff,0xff,0xff,37,0,0,0,0xa5};
  sample_t sample = {.has_sequence=true, .sequence=UINT32_C(0x01020304), .has_tick=true,
    .tick=0, .has_value=true, .value=UINT32_MAX, .has_flags=true, .flags=0xa5};
  uint8_t encoded[20]; size_t length = 0;
  assert(sample_encode(&sample, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(length == sizeof(golden) && memcmp(encoded, golden, sizeof(golden)) == 0);
  for (unsigned mask = 0; mask < 16; ++mask) {
    sample.has_sequence = (mask & 1U) != 0; sample.has_tick = (mask & 2U) != 0;
    sample.has_value = (mask & 4U) != 0; sample.has_flags = (mask & 8U) != 0;
    assert(sample_encoded_size(&sample) == (mask == 15 ? 20 : SIZE_MAX));
    ENCODE_CHECK(sample, sample);
    if (mask != 15) assert(sample_encode(&sample, NULL, 0, NULL) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  }
  for (unsigned r = 0; r < 128; ++r) {
    wide_t wide = {0}; twin_t twin = {0};
    uint32_t f32 = r == 0 ? UINT32_C(0x80000000) : r == 1 ? UINT32_C(0x7fc12345) : random32();
    uint64_t f64 = ((uint64_t)random32() << 32) | random32();
    if (r == 0) f64 = UINT64_C(0x8000000000000000);
    if (r == 1) f64 = UINT64_C(0x7ff8123456789abc);
    wide.a = random32(); wide.b = ((uint64_t)random32() << 32) | random32();
    memcpy(&wide.c, &f32, sizeof(f32)); memcpy(&wide.d, &f64, sizeof(f64));
    twin.first = wide.a; twin.second = wide.b;
    memcpy(&twin.third, &f32, sizeof(f32)); memcpy(&twin.fourth, &f64, sizeof(f64));
    for (unsigned mask = 0; mask < 16; ++mask) {
      wide.has_a = twin.has_first = (mask & 1U) != 0;
      wide.has_b = twin.has_second = (mask & 2U) != 0;
      wide.has_c = twin.has_third = (mask & 4U) != 0;
      wide.has_d = twin.has_fourth = (mask & 8U) != 0;
      ENCODE_CHECK(wide, wide); ENCODE_CHECK(twin, twin);
    }
  }
  optional_t optional = {.has_a=true,.a=1,.has_b=false}; ENCODE_CHECK(optional, optional);
  optional.has_b=true; optional.b=UINT32_MAX; ENCODE_CHECK(optional, optional);
  variable_t variable = {.has_a=true,.a=UINT32_MAX,.has_b=true,.b=1}; ENCODE_CHECK(variable, variable);
  return 0;
}
