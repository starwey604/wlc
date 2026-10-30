/* SPDX-License-Identifier: Apache-2.0 */
#include "owned.h"
#include <stdio.h>
#include <string.h>
#ifdef WLC_VALUE_BENCHMARK
#include <time.h>
#endif

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)

/* Differential error precedence, canonical output, and failure atomicity for
 * the shared generic traversal and the scalar backend, including malformed TLV. */
#define DIFFERENTIAL(name, maximum) \
static int check_##name(const uint8_t *wire, size_t size) { \
  name##_t view; name##_value_t owned, before; \
  uint8_t a[maximum], b[maximum]; size_t an, bn; \
  memset(&owned, 0xA5, sizeof(owned)); memcpy(&before, &owned, sizeof(before)); \
  wl_codec_status_t status = name##_decode(wire, size, &view); \
  CHECK(name##_value_decode(wire, size, &owned) == status); \
  if (status == WL_CODEC_OK) { \
    CHECK(name##_encode(&view, a, sizeof(a), &an) == WL_CODEC_OK); \
    CHECK(name##_value_encode(&owned, b, sizeof(b), &bn) == WL_CODEC_OK); \
    CHECK(an == bn && memcmp(a, b, an) == 0); \
  } else CHECK(memcmp(&owned, &before, sizeof(owned)) == 0); \
  return 0; \
} \
static int mutate_##name(uint8_t *wire, size_t size) { \
  for (size_t n = 0; n <= size; ++n) CHECK(check_##name(wire, n) == 0); \
  for (size_t at = 0; at < size; ++at) { \
    uint8_t saved = wire[at]; \
    for (unsigned byte = 0; byte <= UINT8_MAX; ++byte) { \
      wire[at] = (uint8_t)byte; CHECK(check_##name(wire, size) == 0); \
    } \
    wire[at] = saved; \
  } \
  return 0; \
}
DIFFERENTIAL(snapshot, SNAPSHOT_MAX_ENCODED_SIZE)
DIFFERENTIAL(scalars, SCALARS_MAX_ENCODED_SIZE)

#ifdef WLC_VALUE_BENCHMARK
static int benchmark(void) {
  static large_value_t large;
  large_t view;
  uint8_t wire[LARGE_MAX_ENCODED_SIZE];
  size_t size;
  const unsigned count = 100000U;
  large_value_clear(&large);
  large.has_payload = true;
  large.payload.length = 2031U;
  memset(large.payload.data, 0xA5, large.payload.length);
  CHECK(large_value_encode(&large, wire, sizeof(wire), &size) == WL_CODEC_OK);
  for (unsigned owned = 0U; owned < 2U; ++owned) {
    const clock_t start = clock();
    for (unsigned i = 0U; i < count; ++i) {
      CHECK((owned ? large_value_decode(wire, size, &large)
                   : large_decode(wire, size, &view)) == WL_CODEC_OK);
    }
    const clock_t finish = clock();
    CHECK(start != (clock_t)-1 && finish != (clock_t)-1);
    printf("host CPU large %s_decode payload=2031 iterations=%u ns/op=%.2f\n",
        owned ? "value" : "view", count,
        (double)(finish - start) * 1e9 / (double)CLOCKS_PER_SEC / count);
  }
  return 0;
}
#endif

int main(void) {
  {
    scalars_value_t scalar; uint8_t encoded[SCALARS_MAX_ENCODED_SIZE]; size_t length;
    scalars_value_clear(&scalar);
    scalar.has_value = scalar.has_wide = scalar.has_real = true;
    scalar.value = UINT32_MAX; scalar.wide = UINT64_MAX; scalar.real = -0.0;
    CHECK(scalars_value_encode(&scalar, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
    CHECK(mutate_scalars(encoded, length) == 0);
  }

  uint8_t wire[SNAPSHOT_MAX_ENCODED_SIZE], again[sizeof(wire)];
  snapshot_value_t value, saved, decoded, before;
  snapshot_t view;
  size_t size = 0U, other_size = 0U;
  static const char embedded[] = {'A', '\0', (char)0xC3, (char)0xA9};
  snapshot_value_clear(&value);
  CHECK(!value.has_device && !value.device.has_greeting);
  CHECK(value.device.greeting.length == 3U && value.device.greeting.data[3] == '\0');
  CHECK(memcmp(value.device.greeting.data, "\xE7\x94\xB5", 3U) == 0);
  CHECK(value.device.state == STOPPED && !value.device.has_state);
  value.has_device = value.has_samples = true;
  value.device.has_name = value.device.has_serial = true;
  memcpy(value.device.name.data, embedded, sizeof(embedded));
  value.device.name.length = sizeof(embedded);
  value.device.serial.length = 4U;
  memset(value.device.serial.data, 0xAD, 4U);
  value.samples[0] = 1.25F;
  value.samples[1] = -0.0F;
  value.samples[2] = 42.0F;
  CHECK(snapshot_value_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(snapshot_value_encoded_size(&value) == size);
  CHECK(mutate_snapshot(wire, size) == 0);
  CHECK(snapshot_decode(wire, size, &view) == WL_CODEC_OK);
  CHECK(view.device.name.data >= (const char *)wire && view.device.name.data < (const char *)(wire + size));
  CHECK(snapshot_encode(&view, again, sizeof(again), &other_size) == WL_CODEC_OK);
  CHECK(size == other_size && memcmp(wire, again, size) == 0);
  CHECK(snapshot_value_from_view(&view, &decoded) == WL_CODEC_OK);
  CHECK(snapshot_value_decode(wire, size, &saved) == WL_CODEC_OK);
  before = decoded;
  CHECK(snapshot_value_to_view(&decoded, &view) == WL_CODEC_OK);
  CHECK(view.device.name.data == decoded.device.name.data);
  CHECK(snapshot_encode(&view, again, sizeof(again), &other_size) == WL_CODEC_OK);
  CHECK(size == other_size && memcmp(wire, again, size) == 0);
  saved = decoded; /* No hidden pointers into decoded or wire. */
  memset(&decoded, 0xEE, sizeof(decoded));
  memset(wire, 0xEE, sizeof(wire));
  CHECK(saved.device.name.length == sizeof(embedded));
  CHECK(memcmp(saved.device.name.data, embedded, sizeof(embedded)) == 0);
  CHECK(saved.device.name.data[sizeof(embedded)] == '\0');
  CHECK(saved.device.serial.data[3] == 0xADU && saved.samples[2] == 42.0F);
  CHECK(snapshot_value_encode(&saved, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(size == other_size && memcmp(wire, again, size) == 0);

  /* All failures are transactional at the value boundary. */
  decoded = before;
  CHECK(snapshot_value_decode(wire, 0U, &decoded) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  CHECK(memcmp(&before, &decoded, sizeof(before)) == 0);
  CHECK(snapshot_value_decode(wire, size - 1U, &decoded) != WL_CODEC_OK);
  CHECK(memcmp(&before, &decoded, sizeof(before)) == 0);
  value.device.name.length = 32U;
  CHECK(snapshot_value_encoded_size(&value) == SIZE_MAX);
  CHECK(snapshot_value_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_ERR_INVALID_VALUE);
  value.device.name.length = 2U;
  value.device.name.data[0] = (char)0xC0;
  value.device.name.data[1] = (char)0x80;
  CHECK(snapshot_value_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_ERR_UTF8);
  CHECK(snapshot_value_to_view(&saved, &view) == WL_CODEC_OK);
  view.device.name.length = 32U;
  CHECK(snapshot_value_from_view(&view, &decoded) == WL_CODEC_ERR_INVALID_VALUE);
  CHECK(memcmp(&before, &decoded, sizeof(before)) == 0);
  /* Absent fields do not cause reads through stale pointers or lengths. */
  view.device.has_name = true;
  view.device.name.length = 0U;
  view.device.name.data = NULL;
  view.device.has_serial = view.device.has_greeting = false;
  view.device.serial.data = NULL;
  view.device.serial.length = SIZE_MAX;
  view.device.greeting.data = NULL;
  view.device.greeting.length = SIZE_MAX;
  CHECK(snapshot_value_from_view(&view, &decoded) == WL_CODEC_OK);
  CHECK(decoded.device.name.length == 0U && decoded.device.serial.length == 0U);
  CHECK(decoded.device.greeting.length == 3U && !decoded.device.has_greeting);
  memset(value.device.name.data, 'x', 31U);
  value.device.name.length = 31U;
  CHECK(snapshot_value_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(snapshot_value_decode(wire, size, &decoded) == WL_CODEC_OK);
  CHECK(decoded.device.name.length == 31U && decoded.device.name.data[31] == '\0');

  {
    static large_value_t large, copy;
    uint8_t big_wire[LARGE_MAX_ENCODED_SIZE];
    large_value_clear(&large);
    large.has_payload = true;
    large.payload.length = sizeof(large.payload.data);
    for (size_t i = 0U; i < large.payload.length; ++i) large.payload.data[i] = (uint8_t)i;
    CHECK(large_value_encode(&large, big_wire, sizeof(big_wire), &size) == WL_CODEC_OK);
    CHECK(size == LARGE_MAX_ENCODED_SIZE);
    CHECK(large_value_decode(big_wire, size, &copy) == WL_CODEC_OK);
    memset(big_wire, 0, sizeof(big_wire));
    memset(&large, 0, sizeof(large));
    CHECK(copy.payload.length == 2031U && copy.payload.data[2030] == (uint8_t)2030U);
    copy.payload.length = 0U;
    CHECK(large_value_encode(&copy, big_wire, sizeof(big_wire), &size) == WL_CODEC_OK);
    CHECK(size == 2U);
  }
  {
    empty_value_t empty;
    CHECK(empty_value_decode(NULL, 0U, &empty) == WL_CODEC_OK);
    CHECK(empty_value_encoded_size(&empty) == 0U);
  }
  printf("owned x86_64 sizes: DeviceInfo=%zu Snapshot=%zu Large=%zu; encoded maxima=%llu/%llu/%llu\n",
      sizeof(device_info_value_t), sizeof(snapshot_value_t), sizeof(large_value_t),
      (unsigned long long)DEVICE_INFO_MAX_ENCODED_SIZE,
      (unsigned long long)SNAPSHOT_MAX_ENCODED_SIZE, (unsigned long long)LARGE_MAX_ENCODED_SIZE);
#ifdef WLC_VALUE_BENCHMARK
  CHECK(benchmark() == 0);
#endif
  return 0;
}
