/* SPDX-License-Identifier: Apache-2.0 */
#include <assert.h>
#include "plan.c"
static uint32_t random_state = UINT32_C(0xa50b391d);
static uint32_t next_random(void) {
  random_state ^= random_state << 13; random_state ^= random_state >> 17;
  random_state ^= random_state << 5; return random_state;
}
#define TEST(name, is_required) do { \
  name##_t input, a, b, canary; \
  uint8_t wire[2048], actual[2048], mutated[2048]; \
  size_t length = 0; \
  name##_clear(NULL); \
  memset(&a, 0xA5, sizeof(a)); \
  name##_clear(&a); \
  assert(!a.has_values); \
  name##_clear(&input); input.has_values = true; \
  for (size_t i = 0; i < sizeof(input.values); ++i) ((uint8_t *)input.values)[i] = (uint8_t)next_random(); \
  assert(name##_encode(&input, wire, sizeof(wire), &length) == WL_CODEC_OK); \
  assert(name##_encoded_size(&input) == length); \
  for (size_t cap = 0; cap <= length + 1; ++cap) { \
    size_t an = 9999; \
    memset(actual, 0xA5, sizeof(actual)); \
    int ar = name##_encode(&input, actual, cap, &an); \
    if (cap < length) { \
      assert(ar == WL_CODEC_ERR_CAPACITY); \
    } else { \
      assert(ar == WL_CODEC_OK && an == length && memcmp(actual, wire, length) == 0); \
    } \
  } \
  for (size_t n = 0; n <= length; ++n) { \
    memset(&a, 0xA5, sizeof(a)); canary = a; \
    int ar = name##_decode(wire, n, &a); \
    if (n == length) { \
      assert(ar == WL_CODEC_OK && a.has_values && memcmp(a.values, input.values, sizeof(a.values)) == 0); \
    } else if (n == 0 && !(is_required)) { \
      assert(ar == WL_CODEC_OK && !a.has_values); \
    } else { \
      assert(ar != WL_CODEC_OK && memcmp(&a, &canary, sizeof(a)) == 0); \
    } \
  } \
  for (unsigned r = 0; r < 4096; ++r) { \
    memcpy(mutated, wire, length); \
    size_t n = length; \
    switch (r % 4) { \
      case 0: for (unsigned i = 0; i < 4; ++i) mutated[next_random() % length] = (uint8_t)next_random(); break; \
      case 1: memcpy(mutated + n, wire, length); n += length; break; \
      case 2: mutated[n++] = 0x78; mutated[n++] = 42; break; \
      default: n = next_random() % 64; for (size_t i = 0; i < n; ++i) mutated[i] = (uint8_t)next_random(); break; \
    } \
    memset(&a, 0xA5, sizeof(a)); canary = a; \
    int ar = name##_decode(mutated, n, &a); \
    if (ar != WL_CODEC_OK) { \
      assert(memcmp(&a, &canary, sizeof(a)) == 0); \
    } else if (a.has_values) { \
      size_t bl = 0; \
      assert(name##_encode(&a, actual, sizeof(actual), &bl) == WL_CODEC_OK); \
      assert(name##_decode(actual, bl, &b) == WL_CODEC_OK); \
      assert(memcmp(a.values, b.values, sizeof(a.values)) == 0); \
    } \
  } \
  input.has_values = false; \
  size_t an = 9999; \
  int absent_status = name##_encode(&input, actual, sizeof(actual), &an); \
  if (is_required) { \
    assert(absent_status == WL_CODEC_ERR_MISSING_REQUIRED_FIELD); \
  } else { \
    assert(absent_status == WL_CODEC_OK && an == 0U); \
  } \
  memset(&a, 0xA5, sizeof(a)); \
  assert(name##_decode(NULL, 0, &a) == ((is_required) ? WL_CODEC_ERR_MISSING_REQUIRED_FIELD : WL_CODEC_OK)); \
  assert(name##_decode(NULL, 1, &a) == WL_CODEC_ERR_INVALID_VALUE); \
  assert(name##_decode(wire, length, NULL) == WL_CODEC_ERR_INVALID_VALUE); \
  assert(name##_encode(NULL, actual, sizeof(actual), &length) == WL_CODEC_ERR_INVALID_VALUE); \
} while (0)
int main(void) {
  TEST(packed, 0); TEST(wide, 1); TEST(fixed, 0); TEST(one, 1);
  return 0;
}
