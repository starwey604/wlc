use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

fn wirelink_include() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("include")
}

#[test]
fn generated_bounded_views_are_zero_copy_and_enforce_byte_lengths() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
message Bounded = 1 {
  required string<4> name = 1;
  optional bytes<3> raw = 2;
  repeated string<2> labels = 3;
  optional string<3> greeting = 4 [default = "电"];
}
message Fixed = 2 {
  required string<3> name = 1;
  optional bytes<128> blob = 16;
}
message Nested = 3 {
  required Fixed fixed = 1;
  optional string<255> note = 2;
}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "bounded_lengths").unwrap();
    for declaration in [
        "wl_codec_string_t name;",
        "wl_codec_bytes_t raw;",
        "wl_codec_string_t *labels;",
    ] {
        assert!(
            generated.header.contains(declaration),
            "missing borrowed-view declaration `{declaration}`"
        );
    }

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("bounded_lengths.h"), generated.header).unwrap();
    fs::write(directory.path().join("bounded_lengths.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "bounded_lengths.h"

#include <stdint.h>
#include <string.h>

static const uint8_t golden[] = {
  0x0AU, 0x04U, 'f', 'o', 'u', 'r',
  0x12U, 0x03U, 0x01U, 0x02U, 0x03U,
  0x1AU, 0x02U, 'o', 'k',
  0x1AU, 0x02U, 0xC3U, 0xA9U
};

_Static_assert(FIXED_HAS_MAX_ENCODED_SIZE == 1,
               "bounded scalar views have a static maximum");
_Static_assert(FIXED_MAX_ENCODED_SIZE == UINT64_C(137),
               "bounded scalar maximum");
_Static_assert(NESTED_MAX_ENCODED_SIZE == UINT64_C(398),
               "nested bounded maximum");
_Static_assert(BOUNDED_HAS_MAX_ENCODED_SIZE == 0,
               "a bounded repeated element still has unbounded count");

static int test_valid_and_borrowed(void) {
  static const uint8_t raw[] = {0x01U, 0x02U, 0x03U};
  wl_codec_string_t labels[] = {{"ok", 2U}, {"\xC3\xA9", 2U}};
  wl_codec_string_t decoded_labels[2] = {{0}};
  bounded_t value = {0};
  bounded_t decoded = {0};
  uint8_t encoded[sizeof(golden)];
  size_t length = 0U;

  bounded_clear(&value);
  if (value.has_greeting || value.greeting.length != 3U ||
      memcmp(value.greeting.data, "\xE7\x94\xB5", 3U) != 0) return 1;
  value.has_name = true;
  value.name = (wl_codec_string_t){"four", 4U};
  value.has_raw = true;
  value.raw = (wl_codec_bytes_t){raw, sizeof(raw)};
  value.labels = labels;
  value.labels_count = 2U;
  value.labels_capacity = 2U;
  if (bounded_encoded_size(&value) != sizeof(golden)) return 2;
  if (bounded_encode(&value, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != sizeof(golden) || memcmp(encoded, golden, sizeof(golden)) != 0)
    return 3;

  decoded.labels = decoded_labels;
  decoded.labels_capacity = 2U;
  if (bounded_decode(golden, sizeof(golden), &decoded) != WL_CODEC_OK ||
      !decoded.has_name || decoded.name.length != 4U ||
      decoded.name.data != (const char *)(golden + 2U) ||
      !decoded.has_raw || decoded.raw.length != 3U ||
      decoded.raw.data != golden + 8U || decoded.labels_count != 2U ||
      decoded.labels[0].data != (const char *)(golden + 13U) ||
      decoded.labels[1].data != (const char *)(golden + 17U) ||
      decoded.has_greeting || decoded.greeting.length != 3U) return 4;
  return 0;
}

static int test_encode_limits(void) {
  static const uint8_t raw[] = {1U, 2U, 3U, 4U};
  static const uint8_t invalid_utf8[] = {0xC0U, 0x80U};
  wl_codec_string_t labels[] = {{"bad", 3U}};
  bounded_t value = {0};
  uint8_t encoded[32];
  size_t length = 77U;

  value.has_name = true;
  value.name = (wl_codec_string_t){"abcde", 5U};
  if (bounded_encoded_size(&value) != SIZE_MAX ||
      bounded_encode(&value, encoded, sizeof(encoded), &length) !=
          WL_CODEC_ERR_INVALID_VALUE || length != 77U) return 1;

  value.name = (wl_codec_string_t){"four", 4U};
  value.has_raw = true;
  value.raw = (wl_codec_bytes_t){raw, sizeof(raw)};
  if (bounded_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_INVALID_VALUE) return 2;

  value.has_raw = false;
  value.labels = labels;
  value.labels_count = 1U;
  value.labels_capacity = 1U;
  if (bounded_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_INVALID_VALUE) return 3;

  value.labels_count = 0U;
  value.name = (wl_codec_string_t){(const char *)invalid_utf8,
                                   sizeof(invalid_utf8)};
  if (bounded_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_UTF8) return 4;
  return 0;
}

static int test_decode_limits(void) {
  static const uint8_t name_too_long[] = {
    0x0AU, 0x05U, 'a', 'b', 'c', 'd', 'e'
  };
  static const uint8_t bound_before_truncation[] = {0x0AU, 0x05U};
  static const uint8_t raw_too_long[] = {
    0x12U, 0x04U, 1U, 2U, 3U, 4U
  };
  static const uint8_t label_too_long[] = {0x1AU, 0x03U, 'b', 'a', 'd'};
  static const uint8_t invalid_utf8[] = {0x0AU, 0x02U, 0xC0U, 0x80U};
  wl_codec_string_t labels[1] = {{0}};
  bounded_t decoded = {0};

  if (bounded_decode(name_too_long, sizeof(name_too_long), &decoded) !=
      WL_CODEC_ERR_INVALID_VALUE) return 1;
  if (bounded_decode(bound_before_truncation, sizeof(bound_before_truncation),
                     &decoded) != WL_CODEC_ERR_INVALID_VALUE) return 2;
  if (bounded_decode(raw_too_long, sizeof(raw_too_long), &decoded) !=
      WL_CODEC_ERR_INVALID_VALUE) return 3;
  decoded.labels = labels;
  decoded.labels_capacity = 1U;
  if (bounded_decode(label_too_long, sizeof(label_too_long), &decoded) !=
      WL_CODEC_ERR_INVALID_VALUE) return 4;
  if (bounded_decode(invalid_utf8, sizeof(invalid_utf8), &decoded) !=
      WL_CODEC_ERR_UTF8) return 5;
  return 0;
}

static int test_static_maxima(void) {
  uint8_t blob[128] = {0};
  char note[255] = {0};
  uint8_t fixed_encoded[FIXED_MAX_ENCODED_SIZE];
  uint8_t nested_encoded[NESTED_MAX_ENCODED_SIZE];
  fixed_t fixed = {0};
  nested_t nested = {0};
  size_t length = 0U;

  fixed.has_name = true;
  fixed.name = (wl_codec_string_t){"max", 3U};
  fixed.has_blob = true;
  fixed.blob = (wl_codec_bytes_t){blob, sizeof(blob)};
  if (fixed_encode(&fixed, fixed_encoded, sizeof(fixed_encoded), &length) !=
          WL_CODEC_OK ||
      length != FIXED_MAX_ENCODED_SIZE) return 1;
  nested.has_fixed = true;
  nested.fixed = fixed;
  nested.has_note = true;
  nested.note = (wl_codec_string_t){note, sizeof(note)};
  if (nested_encode(&nested, nested_encoded, sizeof(nested_encoded), &length) !=
          WL_CODEC_OK ||
      length != NESTED_MAX_ENCODED_SIZE) return 2;
  return 0;
}

int main(void) {
  int result = test_valid_and_borrowed();
  if (result != 0) return result;
  result = test_encode_limits();
  if (result != 0) return 20 + result;
  result = test_decode_limits();
  if (result != 0) return 40 + result;
  result = test_static_maxima();
  return result == 0 ? 0 : 60 + result;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("bounded-lengths-test");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(wirelink_include())
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("bounded_lengths.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "bounded codec must be strict C11-clean");
    assert!(Command::new(executable).status().unwrap().success());

    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "bounded_lengths.h"

#include <type_traits>

static_assert(std::is_same_v<decltype(bounded_t::name), wl_codec_string_t>);
static_assert(std::is_same_v<decltype(bounded_t::raw), wl_codec_bytes_t>);
static_assert(std::is_same_v<decltype(bounded_t::labels),
                             wl_codec_string_t *>);
static_assert(FIXED_MAX_ENCODED_SIZE == UINT64_C(137));
static_assert(NESTED_MAX_ENCODED_SIZE == UINT64_C(398));

int main() { return 0; }
"#,
    )
    .unwrap();
    let cxx_status = Command::new("c++")
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(wirelink_include())
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("header.cpp"))
        .status()
        .unwrap();
    assert!(cxx_status.success(), "bounded header must be C++20-clean");
}
