/* SPDX-License-Identifier: Apache-2.0 */
/* Generated-reader differential contract, including partial failure output. */
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include "candidate.c"
#include "varint_reference.c"

static uint64_t reader_cases, decoder_cases, statuses[3];
static uint32_t random_state = 0x41c64e6dU;

static uint32_t next_random(void) {
  random_state ^= random_state << 13U;
  random_state ^= random_state >> 17U;
  random_state ^= random_state << 5U;
  return random_state;
}

static void reader(const uint8_t *input, size_t length, size_t start) {
  size_t a = start, v = start;
  uint64_t expected = UINT64_C(0xa55adeadbeefc33c), actual = expected;
  wl_codec_status_t sa = reference_getv(input, length, &a, &expected);
  wl_codec_status_t sv = wlc_getv(input, length, &v, &actual);
  assert(sa == sv && a == v && expected == actual);
  assert(sa >= 0 && sa <= WL_CODEC_ERR_OVERFLOW);
  if (sa != WL_CODEC_OK) assert(actual == UINT64_C(0xa55adeadbeefc33c));
  ++reader_cases;
  ++statuses[sa];
}

static void golden(const uint8_t *input, size_t length, size_t start,
                   wl_codec_status_t status, size_t end, uint64_t value) {
  size_t at = start;
  uint64_t out = UINT64_C(0xa55adeadbeefc33c);
  assert(reference_getv(input, length, &at, &out) == status);
  assert(at == end && out == value);
  reader(input, length, start);
}

static size_t put(uint8_t *out, uint64_t value) {
  size_t n = 0U;
  do {
    uint8_t byte = (uint8_t)(value & 127U);
    value >>= 7U;
    out[n++] = (uint8_t)(byte | (value != 0U ? 128U : 0U));
  } while (value != 0U);
  return n;
}

static void encoded(uint64_t value) {
  uint8_t input[16] = {0};
  for (size_t start = 0U; start <= 3U; start += 3U) {
    size_t n = put(input + start, value);
    for (size_t length = start; length <= start + n; ++length)
      reader(input, length, start);
    golden(input, start + n, start, WL_CODEC_OK, start + n, value);
  }
}

static void readers(void) {
  uint8_t input[20] = {0};
  for (unsigned first = 0U; first < 256U; ++first) {
    for (size_t start = 0U; start <= 3U; start += 3U) {
      input[start] = (uint8_t)first;
      for (size_t n = 0U; n <= 1U; ++n) reader(input, start + n, start);
      for (unsigned second = 0U; second < 256U; ++second) {
        input[start + 1U] = (uint8_t)second;
        reader(input, start + 2U, start);
      }
    }
  }
  encoded(0U);
  encoded(UINT64_MAX);
  for (unsigned bit = 1U; bit < 64U; ++bit) {
    uint64_t value = UINT64_C(1) << bit;
    encoded(value - 1U);
    encoded(value);
    encoded(value + 1U);
  }
  for (unsigned i = 0U; i < 10000U; ++i) {
    uint64_t high = next_random(), low = next_random();
    encoded((high << 32U) | low);
  }
  /* Ninth/tenth-byte limits, noncanonical zeros and trailing input. */
  for (unsigned pattern = 0U; pattern < 2U; ++pattern) {
    memset(input, pattern == 0U ? 128 : 255, 9U);
    for (unsigned byte = 0U; byte < 256U; ++byte) {
      input[9] = (uint8_t)byte;
      reader(input, 9U, 0U);
      reader(input, 10U, 0U);
      reader(input, 11U, 0U);
    }
  }
  for (unsigned i = 0U; i < 20000U; ++i) {
    for (size_t j = 0U; j < sizeof(input); ++j) input[j] = (uint8_t)next_random();
    size_t length = next_random() % 17U;
    reader(input, length, next_random() % (length + 2U));
  }
  const uint8_t noncanonical[] = {128U, 0U};
  const uint8_t truncated[] = {128U};
  uint8_t tenth[11];
  memset(tenth, 128, sizeof(tenth));
  tenth[9] = 2U;
  golden(noncanonical, 2U, 0U, WL_CODEC_ERR_MALFORMED, 2U, UINT64_C(0xa55adeadbeefc33c));
  golden(truncated, 1U, 0U, WL_CODEC_ERR_MALFORMED, 1U, UINT64_C(0xa55adeadbeefc33c));
  golden(tenth, 10U, 0U, WL_CODEC_ERR_OVERFLOW, 10U, UINT64_C(0xa55adeadbeefc33c));
  /* No input/output access when empty or already beyond end. */
  golden(NULL, 0U, 0U, WL_CODEC_ERR_MALFORMED, 0U, UINT64_C(0xa55adeadbeefc33c));
  golden(NULL, 0U, SIZE_MAX, WL_CODEC_ERR_OVERFLOW, SIZE_MAX, UINT64_C(0xa55adeadbeefc33c));
  size_t at = 0U;
  assert(wlc_getv(NULL, 0U, &at, NULL) == WL_CODEC_ERR_MALFORMED && at == 0U);
  /* Exact allocations make speculative end reads visible to ASan. */
  for (size_t length = 1U; length <= 11U; ++length) {
    uint8_t *exact = malloc(length);
    assert(exact != NULL);
    memset(exact, 128, length);
    reader(exact, length, 0U);
    reader(exact, length, length);
    exact[length - 1U] = 1U;
    reader(exact, length, length - 1U);
    free(exact);
  }
}

#ifndef READER_ONLY
static void decoder(const uint8_t *input, size_t length) {
  stream_chunk_t a, v;
  memset(&a, 0xa5, sizeof(a));
  memset(&v, 0xa5, sizeof(v));
  wl_codec_status_t sa = reference_decode(&stream_chunk_desc, input, length, &a);
  wl_codec_status_t sv = stream_chunk_decode(input, length, &v);
  /* Includes partial output, flags and untouched padding on every failure. */
  assert(sa == sv && memcmp(&a, &v, sizeof(a)) == 0);
  ++decoder_cases;
}

static void decoders(void) {
  uint8_t input[1040] = {13U, 1U, 2U, 3U, 4U, 18U, 32U};
  for (size_t i = 0U; i < 32U; ++i) input[7U + i] = (uint8_t)i;
  for (size_t length = 0U; length <= 39U; ++length) decoder(input, length);
  for (size_t at = 0U; at < 39U; ++at) {
    uint8_t original = input[at];
    for (unsigned byte = 0U; byte < 256U; ++byte) {
      input[at] = (uint8_t)byte;
      decoder(input, 39U);
    }
    input[at] = original;
  }
  const size_t sizes[] = {0U, 1U, 32U, 127U, 128U, 1024U, 1025U};
  for (size_t i = 0U; i < sizeof(sizes) / sizeof(sizes[0]); ++i) {
    size_t bytes = sizes[i], header = 6U + put(input + 6U, bytes);
    for (size_t n = 0U; n < bytes; ++n) input[header + n] = (uint8_t)n;
    for (size_t length = 6U; length <= header + bytes; ++length) decoder(input, length);
  }
  /* Reordered fields, duplicate sequence, unknown-varint and overwide key. */
  const uint8_t reordered[] = {18U, 1U, 42U, 13U, 1U, 2U, 3U, 4U};
  const uint8_t duplicate[] = {13U, 1U, 2U, 3U, 4U, 13U, 1U, 2U, 3U, 4U};
  const uint8_t unknown[] = {13U, 1U, 2U, 3U, 4U, 24U, 1U, 18U, 0U};
  const uint8_t overwide[] = {128U, 128U, 32U};
  decoder(reordered, sizeof(reordered));
  decoder(duplicate, sizeof(duplicate));
  decoder(unknown, sizeof(unknown));
  decoder(overwide, sizeof(overwide));
  decoder(NULL, 0U);
  for (unsigned i = 0U; i < 10000U; ++i) {
    size_t length = next_random() % 80U;
    for (size_t j = 0U; j < length; ++j) input[j] = (uint8_t)next_random();
    decoder(input, length);
  }
}

#endif

int main(void) {
  readers();
#ifndef READER_ONLY
  decoders();
#endif
  printf("{\"reader_cases\":%" PRIu64 ",\"decoder_cases\":%" PRIu64
         ",\"ok\":%" PRIu64 ",\"malformed\":%" PRIu64 ",\"overflow\":%" PRIu64 "}\n",
         reader_cases, decoder_cases, statuses[0], statuses[1], statuses[2]);
  return 0;
}
