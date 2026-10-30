/* SPDX-License-Identifier: Apache-2.0 */
#include "fp_runtime.c"
#include <assert.h>

int main(void) {
  const uint32_t integers[] = {0U, 1U, 127U, 128U, 16383U, 16384U, UINT32_MAX};
  const uint32_t singles[] = {0U, UINT32_C(0x80000000), UINT32_C(0x7f800000),
      UINT32_C(0xff800000), UINT32_C(0x7fc12345), UINT32_C(0x7fa12345), UINT32_MAX};
  const uint64_t doubles[] = {0U, UINT64_C(0x8000000000000000),
      UINT64_C(0x7ff0000000000000), UINT64_C(0xfff0000000000000),
      UINT64_C(0x7ff812345678abcd), UINT64_C(0x7ff012345678abcd), UINT64_MAX};
  for (unsigned i = 0U; i < 7U; ++i) {
    for (unsigned presence = 0U; presence < 16U; ++presence) {
      scalar_t value, decoded;
      scalar_clear(&value);
      value.has_a = value.has_c = value.has_e = true;
      value.has_b = (presence & 1U) != 0U;
      value.has_d = (presence & 2U) != 0U;
      value.has_f = (presence & 4U) != 0U;
      value.has_g = (presence & 8U) != 0U;
      value.a = integers[i]; value.b = integers[6U-i];
      value.c = ~integers[i]; value.d = doubles[6U-i];
      memcpy(&value.e, &singles[i], sizeof(value.e));
      memcpy(&value.f, &doubles[i], sizeof(value.f));
      value.g = UINT32_MAX;
      uint8_t wire[128], canonical[128]; size_t size, length, hashed;
      assert(fp_rpc_scalar_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
      assert(fp_rpc_scalar_decode(wire, size, &decoded) == WL_CODEC_OK);
      assert(memcmp(&decoded.e, &singles[i], sizeof(decoded.e)) == 0);
      if (decoded.has_f) assert(memcmp(&decoded.f, &doubles[i], sizeof(decoded.f)) == 0);
      assert(scalar_encode(&decoded, canonical, sizeof(canonical), &length) == WL_CODEC_OK);
      uint64_t expected = fp_rpc_fingerprint_seed;
      for (size_t j = 0U; j < length; ++j)
        expected = (expected ^ canonical[j]) * UINT64_C(0x00000100000001b3);
      uint64_t actual = fp_rpc_fingerprint_seed;
      assert(fp_rpc_scalar_wlc_detail_fingerprint(&decoded, &actual, &hashed) == WL_CODEC_OK);
      assert(actual == expected && hashed == length);
    }
  }
  empty_t empty, decoded;
  empty_clear(&empty);
  uint8_t wire[32], canonical[32];
  for (unsigned present = 0U; present < 2U; ++present) {
    empty.has_value = present != 0U;
    size_t size, length, hashed;
    assert(fp_rpc_empty_encode(&empty, wire, sizeof(wire), &size) == WL_CODEC_OK);
    assert(fp_rpc_empty_decode(wire, size, &decoded) == WL_CODEC_OK);
    assert(empty_encode(&decoded, canonical, sizeof(canonical), &length) == WL_CODEC_OK);
    uint64_t expected = fp_rpc_fingerprint_seed, actual = expected;
    for (size_t i = 0U; i < length; ++i)
      expected = (expected ^ canonical[i]) * UINT64_C(0x00000100000001b3);
    assert(fp_rpc_empty_wlc_detail_fingerprint(&decoded, &actual, &hashed) == WL_CODEC_OK);
    assert(actual == expected && hashed == length);
    assert(present || length == 0U);
  }
  return 0;
}
