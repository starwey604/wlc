/* SPDX-License-Identifier: Apache-2.0 */
#include <assert.h>
#include "plan.c"
static uint32_t random_state = UINT32_C(0xa50b391d);
static uint32_t next_random(void) {
  random_state ^= random_state << 13; random_state ^= random_state >> 17;
  random_state ^= random_state << 5; return random_state;
}
#define TEST(name) do { \
  name##_t input, a, b; \
  uint8_t wire[2048], actual[2048], expected[2048], mutated[2048]; \
  size_t length; \
  name##_clear(NULL); \
  memset(&a, 0xA5, sizeof(a)); memset(&b, 0xA5, sizeof(b)); \
  name##_clear(&a); wlc_clear(&name##_desc, &b); \
  assert(memcmp(&a, &b, sizeof(a)) == 0); \
  name##_clear(&input); input.has_values = true; \
  for (size_t i = 0; i < sizeof(input.values); ++i) ((uint8_t *)input.values)[i] = (uint8_t)next_random(); \
  assert(wlc_encode(&name##_desc, &input, wire, sizeof(wire), &length) == WL_CODEC_OK); \
  assert(name##_encoded_size(&input) == length); \
  for (size_t cap = 0; cap <= length + 1; ++cap) { \
    size_t an = 9999, bn = 9999; \
    memset(actual, 0xA5, sizeof(actual)); memset(expected, 0xA5, sizeof(expected)); \
    int ar = name##_encode(&input, actual, cap, &an); \
    int br = wlc_encode(&name##_desc, &input, expected, cap, &bn); \
    assert(ar == br && an == bn && memcmp(actual, expected, sizeof(actual)) == 0); \
  } \
  for (size_t n = 0; n <= length; ++n) { \
    memset(&a, 0xA5, sizeof(a)); memset(&b, 0xA5, sizeof(b)); \
    int ar = name##_decode(wire, n, &a), br = wlc_decode(&name##_desc, wire, n, &b); \
    assert(ar == br && memcmp(&a, &b, sizeof(a)) == 0); \
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
    memset(&a, 0xA5, sizeof(a)); memset(&b, 0xA5, sizeof(b)); \
    int ar = name##_decode(mutated, n, &a), br = wlc_decode(&name##_desc, mutated, n, &b); \
    assert(ar == br && memcmp(&a, &b, sizeof(a)) == 0); \
  } \
  input.has_values = false; \
  for (unsigned p = 0; p < 2; ++p) { \
    size_t an = 9999, bn = 9999; \
    assert(name##_encode(&input, NULL, 0, p ? &an : NULL) == \
        wlc_encode(&name##_desc, &input, NULL, 0, p ? &bn : NULL)); \
    assert(an == bn); \
  } \
  memset(&a, 0xA5, sizeof(a)); memset(&b, 0xA5, sizeof(b)); \
  assert(name##_decode(NULL, 0, &a) == wlc_decode(&name##_desc, NULL, 0, &b)); \
  assert(memcmp(&a, &b, sizeof(a)) == 0); \
  assert(name##_decode(NULL, 1, &a) == WL_CODEC_ERR_INVALID_VALUE); \
  assert(name##_decode(wire, length, NULL) == WL_CODEC_ERR_INVALID_VALUE); \
  assert(name##_encode(NULL, actual, sizeof(actual), &length) == WL_CODEC_ERR_INVALID_VALUE); \
} while (0)
int main(void) {
  TEST(packed); TEST(wide); TEST(fixed); TEST(one);
  return 0;
}
