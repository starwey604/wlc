use std::{fs, process::Command};

use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

#[test]
fn generated_c_compiles_and_round_trips_scalars_repeated_and_nested_messages() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
            enum Mode = 1 { OFF = 0; ON = 1; }
            message Child = 2 { optional int32 delta = 1; }
            message Packet = 3 {
              optional bool ready = 1;
              optional uint64 timestamp = 2;
              optional fixed32 mask = 3;
              optional string name = 4 [default = "default"];
              optional Mode mode = 5 [default = 0];
              optional Child child = 6;
              repeated uint32 samples = 7;
              optional uint32 maximum = 8;
              optional int64 minimum = 9 [default = -9223372036854775808];
              optional fixed64 wide_mask = 10;
              optional bytes raw = 11;
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "sample").unwrap();
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("sample.h"), generated.header).unwrap();
    fs::write(directory.path().join("sample.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "sample.h"
#include <string.h>

int main(void) {
  uint32_t samples[] = { 3U, 9U };
  uint32_t decoded_samples[2] = { 0U, 0U };
  const uint8_t raw[] = { 0x00U, 0xFFU };
  uint8_t bytes[128];
  size_t length = 0U;
  packet_t packet = {0};
  packet_clear(&packet);
  packet.has_ready = true; packet.ready = true;
  packet.has_timestamp = true; packet.timestamp = UINT64_C(18446744073709551615);
  packet.has_mask = true; packet.mask = UINT32_C(0x01020304);
  packet.has_name = true; packet.name = (wl_codec_string_t){ "ok", 2U };
  packet.has_mode = true; packet.mode = ON;
  packet.has_child = true; child_clear(&packet.child); packet.child.has_delta = true; packet.child.delta = -42;
  packet.samples = samples; packet.samples_count = 2U; packet.samples_capacity = 2U;
  packet.has_maximum = true; packet.maximum = UINT32_MAX;
  packet.has_minimum = true; packet.minimum = INT64_MIN;
  packet.has_wide_mask = true; packet.wide_mask = UINT64_C(0x0102030405060708);
  packet.has_raw = true; packet.raw = (wl_codec_bytes_t){ raw, sizeof(raw) };
  if (packet_encode(&packet, bytes, sizeof(bytes), &length) != WL_CODEC_OK) return 1;
  if (bytes[0] != 0x08U || bytes[2] != 0x10U) return 12; /* field-number order */
  packet_t decoded = {0}; decoded.samples = decoded_samples; decoded.samples_capacity = 2U;
  if (packet_decode(bytes, length, &decoded) != WL_CODEC_OK) return 2;
  if (!decoded.has_ready || !decoded.ready || !decoded.has_timestamp || decoded.timestamp != packet.timestamp) return 3;
  if (!decoded.has_mask || decoded.mask != packet.mask || !decoded.has_name || decoded.name.length != 2U) return 4;
  if (!decoded.has_mode || decoded.mode != ON || !decoded.has_child || decoded.child.delta != -42) return 5;
  if (decoded.samples_count != 2U || decoded.samples[0] != 3U || decoded.samples[1] != 9U) return 6;
  if (!decoded.has_maximum || decoded.maximum != UINT32_MAX || !decoded.has_minimum || decoded.minimum != INT64_MIN) return 13;
  if (!decoded.has_wide_mask || decoded.wide_mask != packet.wide_mask || !decoded.has_raw || decoded.raw.length != sizeof(raw)) return 14;
  bytes[length++] = 0x78U; bytes[length++] = 0x01U; /* unknown field 15 */
  decoded.samples = decoded_samples; decoded.samples_capacity = 2U;
  if (packet_decode(bytes, length, &decoded) != WL_CODEC_OK) return 7;
  { const uint8_t duplicate[] = { 0x08U, 0x01U, 0x08U, 0x00U };
    if (packet_decode(duplicate, sizeof(duplicate), &decoded) != WL_CODEC_ERR_DUPLICATE_FIELD) return 8; }
  { const uint8_t capacity[] = { 0x38U, 0x01U, 0x38U, 0x02U };
    decoded.samples = decoded_samples; decoded.samples_capacity = 1U;
    if (packet_decode(capacity, sizeof(capacity), &decoded) != WL_CODEC_ERR_CAPACITY) return 9; }
  { const uint8_t invalid_utf8[] = { 0x22U, 0x02U, 0xC0U, 0x80U };
    if (packet_decode(invalid_utf8, sizeof(invalid_utf8), &decoded) != WL_CODEC_ERR_UTF8) return 10; }
  { const uint8_t malformed[] = { 0x80U };
    if (packet_decode(malformed, sizeof(malformed), &decoded) != WL_CODEC_ERR_MALFORMED) return 11; }
  return 0;
}

"#,
    )
    .unwrap();
    let include = env!("CARGO_MANIFEST_DIR").replace("/wlc", "/include");
    let executable = directory.path().join("codec-test");
    let status = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(include)
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("sample.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "generated C must compile cleanly");
    assert!(Command::new(executable).status().unwrap().success());
}

#[test]
fn generated_c_preserves_dense_numeric_golden_bits_and_validates_packed_lengths() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
            message Dense = 1 {
              optional float32 scalar32 = 1;
              optional float64 scalar64 = 2;
              packed float32 values32[4] = 3;
              packed float64 values64[4] = 4;
              packed fixed32 words32[2] = 5;
              packed fixed64 words64[2] = 6;
              packed float32 control[30] = 7;
              repeated float32 samples = 8;
            }
            message Envelope = 2 { optional Dense child = 1; }"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "dense_numeric").unwrap();
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("dense_numeric.h"), generated.header).unwrap();
    fs::write(directory.path().join("dense_numeric.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "dense_numeric.h"
#include <stdint.h>
#include <string.h>

static float float_from_bits(uint32_t bits) {
  float value;
  memcpy(&value, &bits, sizeof(value));
  return value;
}
static double double_from_bits(uint64_t bits) {
  double value;
  memcpy(&value, &bits, sizeof(value));
  return value;
}
static uint32_t float_bits(float value) {
  uint32_t bits;
  memcpy(&bits, &value, sizeof(bits));
  return bits;
}
static uint64_t double_bits(double value) {
  uint64_t bits;
  memcpy(&bits, &value, sizeof(bits));
  return bits;
}

int main(void) {
  static const uint8_t golden[] = {
    0x0DU, 0x7FU, 0xC1U, 0x23U, 0x45U,
    0x11U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x01U,
    0x1AU, 0x10U,
      0x00U, 0x00U, 0x00U, 0x00U,
      0x80U, 0x00U, 0x00U, 0x00U,
      0x7FU, 0x7FU, 0xFFU, 0xFFU,
      0x7FU, 0xC0U, 0x00U, 0x42U,
    0x22U, 0x20U,
      0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U,
      0x80U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U,
      0x7FU, 0xEFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU,
      0x7FU, 0xF8U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x42U,
    0x2AU, 0x08U,
      0x00U, 0x00U, 0x00U, 0x00U, 0xFFU, 0xFFU, 0xFFU, 0xFFU,
    0x32U, 0x10U,
      0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U, 0x00U,
      0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0xFFU
  };
  uint8_t bytes[256];
  size_t length = 0U;
  dense_t value = {0};
  dense_clear(&value);
  value.has_scalar32 = true;
  value.scalar32 = float_from_bits(UINT32_C(0x7FC12345));
  value.has_scalar64 = true;
  value.scalar64 = double_from_bits(UINT64_C(0x0000000000000001));
  value.has_values32 = true;
  value.values32[0] = float_from_bits(UINT32_C(0x00000000));
  value.values32[1] = float_from_bits(UINT32_C(0x80000000));
  value.values32[2] = float_from_bits(UINT32_C(0x7F7FFFFF));
  value.values32[3] = float_from_bits(UINT32_C(0x7FC00042));
  value.has_values64 = true;
  value.values64[0] = double_from_bits(UINT64_C(0x0000000000000000));
  value.values64[1] = double_from_bits(UINT64_C(0x8000000000000000));
  value.values64[2] = double_from_bits(UINT64_C(0x7FEFFFFFFFFFFFFF));
  value.values64[3] = double_from_bits(UINT64_C(0x7FF8000000000042));
  value.has_words32 = true;
  value.words32[0] = 0U;
  value.words32[1] = UINT32_MAX;
  value.has_words64 = true;
  value.words64[0] = 0U;
  value.words64[1] = UINT64_MAX;
  if (dense_encoded_size(&value) != sizeof(golden)) return 1;
  if (dense_encode(&value, bytes, sizeof(bytes), &length) != WL_CODEC_OK) return 2;
  if (length != sizeof(golden) || memcmp(bytes, golden, sizeof(golden)) != 0) return 3;

  dense_t decoded = {0};
  if (dense_decode(golden, sizeof(golden), &decoded) != WL_CODEC_OK) return 4;
  if (!decoded.has_scalar32 || float_bits(decoded.scalar32) != UINT32_C(0x7FC12345)) return 5;
  if (!decoded.has_scalar64 || double_bits(decoded.scalar64) != UINT64_C(0x0000000000000001)) return 6;
  if (!decoded.has_values32 ||
      float_bits(decoded.values32[0]) != UINT32_C(0x00000000) ||
      float_bits(decoded.values32[1]) != UINT32_C(0x80000000) ||
      float_bits(decoded.values32[2]) != UINT32_C(0x7F7FFFFF) ||
      float_bits(decoded.values32[3]) != UINT32_C(0x7FC00042)) return 7;
  if (!decoded.has_values64 ||
      double_bits(decoded.values64[0]) != UINT64_C(0x0000000000000000) ||
      double_bits(decoded.values64[1]) != UINT64_C(0x8000000000000000) ||
      double_bits(decoded.values64[2]) != UINT64_C(0x7FEFFFFFFFFFFFFF) ||
      double_bits(decoded.values64[3]) != UINT64_C(0x7FF8000000000042)) return 8;
  if (!decoded.has_words32 || decoded.words32[0] != 0U || decoded.words32[1] != UINT32_MAX ||
      !decoded.has_words64 || decoded.words64[0] != 0U || decoded.words64[1] != UINT64_MAX) return 9;

  dense_clear(&value);
  value.has_control = true;
  if (dense_encoded_size(&value) != 122U) return 10;
  if (dense_encode(&value, bytes, sizeof(bytes), &length) != WL_CODEC_OK ||
      length != 122U || bytes[0] != 0x3AU || bytes[1] != 0x78U) return 11;

  envelope_t envelope = {0};
  envelope_clear(&envelope);
  envelope.has_child = true;
  envelope.child.has_control = true;
  if (envelope_encoded_size(&envelope) != 124U) return 12;
  if (envelope_encode(&envelope, bytes, sizeof(bytes), &length) != WL_CODEC_OK) return 13;
  envelope_t decoded_envelope = {0};
  if (envelope_decode(bytes, length, &decoded_envelope) != WL_CODEC_OK ||
      !decoded_envelope.has_child || !decoded_envelope.child.has_control) return 14;

  {
    float samples[] = { float_from_bits(UINT32_C(0x80000000)),
                        float_from_bits(UINT32_C(0x7FC0ABCD)) };
    float decoded_samples[2] = {0.0F, 0.0F};
    dense_t repeated = {0};
    repeated.samples = samples;
    repeated.samples_capacity = 2U;
    dense_clear(&repeated);
    repeated.samples_count = 2U;
    if (dense_encode(&repeated, bytes, sizeof(bytes), &length) != WL_CODEC_OK) return 15;
    dense_t repeated_out = {0};
    repeated_out.samples = decoded_samples;
    repeated_out.samples_capacity = 2U;
    if (dense_decode(bytes, length, &repeated_out) != WL_CODEC_OK ||
        repeated_out.samples_count != 2U ||
        float_bits(repeated_out.samples[0]) != UINT32_C(0x80000000) ||
        float_bits(repeated_out.samples[1]) != UINT32_C(0x7FC0ABCD)) return 16;
  }

  {
    const uint8_t short_field[17] = {0x1AU, 0x0FU};
    const uint8_t long_field[19] = {0x1AU, 0x11U};
    const uint8_t truncated[17] = {0x1AU, 0x10U};
    const uint8_t wrong_wire[] = {0x1DU, 0U, 0U, 0U, 0U};
    uint8_t duplicate[36] = {0U};
    duplicate[0] = 0x1AU; duplicate[1] = 0x10U;
    duplicate[18] = 0x1AU; duplicate[19] = 0x10U;
    if (dense_decode(short_field, sizeof(short_field), &decoded) != WL_CODEC_ERR_MALFORMED) return 17;
    if (dense_decode(long_field, sizeof(long_field), &decoded) != WL_CODEC_ERR_MALFORMED) return 18;
    if (dense_decode(truncated, sizeof(truncated), &decoded) != WL_CODEC_ERR_MALFORMED) return 19;
    if (dense_decode(wrong_wire, sizeof(wrong_wire), &decoded) != WL_CODEC_ERR_WIRE_TYPE) return 20;
    if (dense_decode(duplicate, sizeof(duplicate), &decoded) != WL_CODEC_ERR_DUPLICATE_FIELD) return 21;
  }
  return 0;
}
"#,
    )
    .unwrap();
    let include = env!("CARGO_MANIFEST_DIR").replace("/wlc", "/include");
    let executable = directory.path().join("dense-codec-test");
    let status = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(include)
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("dense_numeric.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "generated dense C must compile cleanly");
    assert!(Command::new(executable).status().unwrap().success());
}

#[test]
fn generated_c_compiles_an_empty_message() {
    let model = analyze_schema(&parse_schema("version 1; message Empty = 1 {}").unwrap()).unwrap();
    let generated = generate_c(&model, "empty").unwrap();
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("empty.h"), generated.header).unwrap();
    fs::write(directory.path().join("empty.c"), generated.source).unwrap();
    let include = env!("CARGO_MANIFEST_DIR").replace("/wlc", "/include");
    let status = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(include)
        .arg("-I")
        .arg(directory.path())
        .arg("-fsyntax-only")
        .arg(directory.path().join("empty.c"))
        .status()
        .unwrap();
    assert!(status.success());
}
