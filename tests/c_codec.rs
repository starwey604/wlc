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
