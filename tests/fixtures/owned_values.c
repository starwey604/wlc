/* SPDX-License-Identifier: Apache-2.0 */
#include "owned.h"
#include <stdio.h>
#include <string.h>

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "%d: %s\n", __LINE__, #x); return 1; } } while (0)

#define ATOMICITY(name, maximum) \
static int check_##name(const uint8_t *wire, size_t size) { \
  name##_t owned, before; \
  uint8_t a[maximum]; size_t an; \
  memset(&owned, 0xA5, sizeof(owned)); memcpy(&before, &owned, sizeof(before)); \
  wl_codec_status_t status = name##_decode(wire, size, &owned); \
  if (status == WL_CODEC_OK) { \
    CHECK(name##_encode(&owned, a, sizeof(a), &an) == WL_CODEC_OK); \
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
ATOMICITY(snapshot, SNAPSHOT_MAX_ENCODED_SIZE)
ATOMICITY(scalars, SCALARS_MAX_ENCODED_SIZE)

int main(void) {
  {
    scalars_t scalar; uint8_t encoded[SCALARS_MAX_ENCODED_SIZE]; size_t length;
    scalars_clear(&scalar);
    scalar.has_wide = scalar.has_real = true;
    scalar.value = UINT32_MAX; scalar.wide = UINT64_MAX; scalar.real = -0.0;
    CHECK(scalars_encode(&scalar, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
    CHECK(mutate_scalars(encoded, length) == 0);
  }

  {
    device_info_t info; device_info_clear(&info);
    CHECK(device_info_set_name(&info, "sensor"));
    CHECK(info.name.length == 6U && strcmp(info.name.data, "sensor") == 0);
    CHECK(!device_info_set_greeting(&info, "four"));
    CHECK(!info.has_greeting && info.greeting.length == 3U);
    CHECK(device_info_set_greeting(&info, "hi"));
    CHECK(info.has_greeting && info.greeting.length == 2U);
    const uint8_t data[] = {1U, 2U, 3U, 4U, 5U};
    CHECK(!device_info_set_serial(&info, data, sizeof(data)));
    CHECK(!info.has_serial && info.serial.length == 0U);
    CHECK(device_info_set_serial(&info, data, 4U));
    CHECK(info.has_serial && info.serial.length == 4U && info.serial.data[3] == 4U);
  }

  uint8_t wire[SNAPSHOT_MAX_ENCODED_SIZE], again[sizeof(wire)];
  snapshot_t value, saved, decoded, before;
  size_t size = 0U, other_size = 0U;
  static const char embedded[] = {'A', '\0', (char)0xC3, (char)0xA9};
  snapshot_clear(&value);
  CHECK(!value.device.has_greeting);
  CHECK(value.device.greeting.length == 3U && value.device.greeting.data[3] == '\0');
  CHECK(memcmp(value.device.greeting.data, "\xE7\x94\xB5", 3U) == 0);
  CHECK(value.device.state == STOPPED && !value.device.has_state);
  value.device.has_serial = true;
  memcpy(value.device.name.data, embedded, sizeof(embedded));
  value.device.name.length = sizeof(embedded);
  value.device.serial.length = 4U;
  memset(value.device.serial.data, 0xAD, 4U);
  value.samples[0] = 1.25F;
  value.samples[1] = -0.0F;
  value.samples[2] = 42.0F;
  CHECK(snapshot_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(snapshot_encoded_size(&value) == size);
  CHECK(mutate_snapshot(wire, size) == 0);

  CHECK(snapshot_decode(wire, size, &decoded) == WL_CODEC_OK);
  saved = decoded;
  memset(&decoded, 0xEE, sizeof(decoded));
  memset(wire, 0xEE, sizeof(wire));
  CHECK(saved.device.name.length == sizeof(embedded));
  CHECK(memcmp(saved.device.name.data, embedded, sizeof(embedded)) == 0);
  CHECK(saved.device.name.data[sizeof(embedded)] == '\0');
  CHECK(saved.device.serial.data[3] == 0xADU && saved.samples[2] == 42.0F);
  CHECK(snapshot_encode(&saved, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(snapshot_encode(&saved, again, sizeof(again), &other_size) == WL_CODEC_OK);
  CHECK(size == other_size && memcmp(wire, again, size) == 0);

  before = saved;
  decoded = before;
  CHECK(snapshot_decode(wire, 0U, &decoded) == WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
  CHECK(memcmp(&before, &decoded, sizeof(before)) == 0);
  CHECK(snapshot_decode(wire, size - 1U, &decoded) != WL_CODEC_OK);
  CHECK(memcmp(&before, &decoded, sizeof(before)) == 0);
  value.device.name.length = 32U;
  CHECK(snapshot_encoded_size(&value) == SIZE_MAX);
  CHECK(snapshot_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_ERR_INVALID_VALUE);
  value.device.name.length = 2U;
  value.device.name.data[0] = (char)0xC0;
  value.device.name.data[1] = (char)0x80;
  CHECK(snapshot_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_ERR_UTF8);

  memset(value.device.name.data, 'x', 31U);
  value.device.name.length = 31U;
  CHECK(snapshot_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  CHECK(snapshot_decode(wire, size, &decoded) == WL_CODEC_OK);
  CHECK(decoded.device.name.length == 31U && decoded.device.name.data[31] == '\0');

  {
    static large_t large, copy;
    uint8_t big_wire[LARGE_MAX_ENCODED_SIZE];
    large_clear(&large);
    large.payload.length = sizeof(large.payload.data);
    for (size_t i = 0U; i < large.payload.length; ++i) large.payload.data[i] = (uint8_t)i;
    CHECK(large_encode(&large, big_wire, sizeof(big_wire), &size) == WL_CODEC_OK);
    CHECK(size == LARGE_MAX_ENCODED_SIZE);
    CHECK(large_decode(big_wire, size, &copy) == WL_CODEC_OK);
    memset(big_wire, 0, sizeof(big_wire));
    memset(&large, 0, sizeof(large));
    CHECK(copy.payload.length == 2031U && copy.payload.data[2030] == (uint8_t)2030U);
    copy.payload.length = 0U;
    CHECK(large_encode(&copy, big_wire, sizeof(big_wire), &size) == WL_CODEC_OK);
    CHECK(size == 2U);
  }
  {
    empty_t empty;
    CHECK(empty_decode(NULL, 0U, &empty) == WL_CODEC_OK);
    CHECK(empty_encoded_size(&empty) == 0U);
  }
  return 0;
}
