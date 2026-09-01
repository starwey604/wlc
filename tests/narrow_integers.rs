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
fn generated_narrow_integer_codec_is_exact_and_rejects_out_of_range_varints() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
message Narrow = 1 {
  required uint8 unsigned_8 = 1;
  required uint16 unsigned_16 = 2;
  required int8 signed_8 = 3;
  required int16 signed_16 = 4;
  optional uint8 default_unsigned_8 = 5 [default = 255];
  optional int8 default_signed_8 = 6 [default = -128];
}
message Limits = 2 {
  optional uint8 unsigned_8 = 1;
  optional uint16 unsigned_16 = 2;
  optional int8 signed_8 = 3;
  optional int16 signed_16 = 4;
}
message NarrowList = 3 {
  repeated uint8 values = 1;
  repeated int16 deltas = 2;
}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "narrow_integer").unwrap();
    for declaration in [
        "uint8_t unsigned_8;",
        "uint16_t unsigned_16;",
        "int8_t signed_8;",
        "int16_t signed_16;",
        "uint8_t *values;",
        "int16_t *deltas;",
    ] {
        assert!(
            generated.header.contains(declaration),
            "missing generated declaration `{declaration}`"
        );
    }

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("narrow_integer.h"), generated.header).unwrap();
    fs::write(directory.path().join("narrow_integer.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "narrow_integer.h"

#include <limits.h>
#include <stdint.h>
#include <string.h>

static const uint8_t golden[] = {
  0x08U, 0xFFU, 0x01U,
  0x10U, 0xFFU, 0xFFU, 0x03U,
  0x18U, 0xFFU, 0x01U,
  0x20U, 0xFFU, 0xFFU, 0x03U,
  0x28U, 0xFFU, 0x01U,
  0x30U, 0xFFU, 0x01U
};

_Static_assert(sizeof(((narrow_t *)0)->unsigned_8) == 1U,
               "uint8 storage width");
_Static_assert(sizeof(((narrow_t *)0)->unsigned_16) == 2U,
               "uint16 storage width");
_Static_assert(sizeof(((narrow_t *)0)->signed_8) == 1U,
               "int8 storage width");
_Static_assert(sizeof(((narrow_t *)0)->signed_16) == 2U,
               "int16 storage width");
_Static_assert(NARROW_MAX_ENCODED_SIZE == UINT64_C(20),
               "narrow maximum changed");
_Static_assert(LIMITS_MAX_ENCODED_SIZE == UINT64_C(14),
               "limits maximum changed");

int main(void) {
  uint8_t encoded[64];
  size_t length = 0U;
  narrow_t value = {0};
  narrow_t decoded = {0};

  narrow_clear(&value);
  if (value.has_unsigned_8 || value.has_unsigned_16 || value.has_signed_8 ||
      value.has_signed_16 || value.has_default_unsigned_8 ||
      value.has_default_signed_8 || value.default_unsigned_8 != UINT8_MAX ||
      value.default_signed_8 != INT8_MIN) return 1;
  value.has_unsigned_8 = true;
  value.unsigned_8 = UINT8_MAX;
  value.has_unsigned_16 = true;
  value.unsigned_16 = UINT16_MAX;
  value.has_signed_8 = true;
  value.signed_8 = INT8_MIN;
  value.has_signed_16 = true;
  value.signed_16 = INT16_MIN;
  value.has_default_unsigned_8 = true;
  value.has_default_signed_8 = true;
  if (narrow_encoded_size(&value) != sizeof(golden)) return 2;
  if (narrow_encode(&value, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != sizeof(golden) || memcmp(encoded, golden, sizeof(golden)) != 0)
    return 3;
  if (narrow_decode(golden, sizeof(golden), &decoded) != WL_CODEC_OK ||
      !decoded.has_unsigned_8 || decoded.unsigned_8 != UINT8_MAX ||
      !decoded.has_unsigned_16 || decoded.unsigned_16 != UINT16_MAX ||
      !decoded.has_signed_8 || decoded.signed_8 != INT8_MIN ||
      !decoded.has_signed_16 || decoded.signed_16 != INT16_MIN ||
      !decoded.has_default_unsigned_8 ||
      decoded.default_unsigned_8 != UINT8_MAX ||
      !decoded.has_default_signed_8 || decoded.default_signed_8 != INT8_MIN)
    return 4;

  {
    static const uint8_t unsigned_8_overflow[] = {0x08U, 0x80U, 0x02U};
    static const uint8_t unsigned_16_overflow[] = {
      0x10U, 0x80U, 0x80U, 0x04U
    };
    static const uint8_t signed_8_overflow[] = {0x18U, 0x80U, 0x02U};
    static const uint8_t signed_16_overflow[] = {
      0x20U, 0x80U, 0x80U, 0x04U
    };
    limits_t limits = {0};
    if (limits_decode(unsigned_8_overflow, sizeof(unsigned_8_overflow), &limits) !=
        WL_CODEC_ERR_OVERFLOW) return 5;
    if (limits_decode(unsigned_16_overflow, sizeof(unsigned_16_overflow),
                      &limits) != WL_CODEC_ERR_OVERFLOW) return 6;
    if (limits_decode(signed_8_overflow, sizeof(signed_8_overflow), &limits) !=
        WL_CODEC_ERR_OVERFLOW) return 7;
    if (limits_decode(signed_16_overflow, sizeof(signed_16_overflow), &limits) !=
        WL_CODEC_ERR_OVERFLOW) return 8;
  }

  {
    uint8_t values[] = {0U, UINT8_MAX};
    int16_t deltas[] = {INT16_MIN, INT16_MAX};
    uint8_t decoded_values[2] = {0U};
    int16_t decoded_deltas[2] = {0};
    narrow_list_t list = {0};
    narrow_list_t decoded_list = {0};
    list.values = values;
    list.values_capacity = 2U;
    list.deltas = deltas;
    list.deltas_capacity = 2U;
    narrow_list_clear(&list);
    list.values_count = 2U;
    list.deltas_count = 2U;
    if (narrow_list_encode(&list, encoded, sizeof(encoded), &length) != WL_CODEC_OK)
      return 9;
    decoded_list.values = decoded_values;
    decoded_list.values_capacity = 2U;
    decoded_list.deltas = decoded_deltas;
    decoded_list.deltas_capacity = 2U;
    if (narrow_list_decode(encoded, length, &decoded_list) != WL_CODEC_OK ||
        decoded_list.values_count != 2U || decoded_list.values[0] != 0U ||
        decoded_list.values[1] != UINT8_MAX ||
        decoded_list.deltas_count != 2U ||
        decoded_list.deltas[0] != INT16_MIN ||
        decoded_list.deltas[1] != INT16_MAX) return 10;
  }
  return 0;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("narrow-integer-test");
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
        .arg(directory.path().join("narrow_integer.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "narrow codec must be strict C11-clean");
    assert!(Command::new(executable).status().unwrap().success());

    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "narrow_integer.h"

#include <type_traits>

static_assert(std::is_same_v<decltype(narrow_t::unsigned_8), uint8_t>);
static_assert(std::is_same_v<decltype(narrow_t::unsigned_16), uint16_t>);
static_assert(std::is_same_v<decltype(narrow_t::signed_8), int8_t>);
static_assert(std::is_same_v<decltype(narrow_t::signed_16), int16_t>);
static_assert(NARROW_MAX_ENCODED_SIZE == UINT64_C(20));

int main() {
  narrow_t value{};
  return sizeof(value.unsigned_8) == 1U && sizeof(value.unsigned_16) == 2U
             ? 0
             : 1;
}
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
    assert!(cxx_status.success(), "narrow header must be C++20-clean");
}
