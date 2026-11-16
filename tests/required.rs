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
fn generated_required_fields_use_values_and_require_wire_presence() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
message Child = 1 {
  int32 delta = 1;
  required packed fixed32 axes[2] = 2;
}
message Envelope = 2 {
  uint32 sequence = 1;
  required Child child = 2;
  optional bool enabled = 3;
}
message Packed = 3 { packed fixed32 axes[2] = 1; }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "required_fields").unwrap();
    assert!(!generated.header.contains("bool has_sequence;"));
    assert!(!generated.header.contains("bool has_child;"));
    assert!(!generated.header.contains("bool has_axes;"));
    assert!(generated.header.contains("ENVELOPE_HAS_MAX_ENCODED_SIZE 1"));

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("required_fields.h"), generated.header).unwrap();
    fs::write(directory.path().join("required_fields.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "required_fields.h"

#include <stdint.h>
#include <string.h>

static const uint8_t golden[] = {
  0x08U, 0x96U, 0x01U,
  0x12U, 0x0CU,
    0x08U, 0x01U,
    0x12U, 0x08U,
      0x04U, 0x03U, 0x02U, 0x01U,
      0xDDU, 0xCCU, 0xBBU, 0xAAU
};

_Static_assert(CHILD_MAX_ENCODED_SIZE == UINT64_C(16),
               "required child bound changed");
_Static_assert(ENVELOPE_MAX_ENCODED_SIZE == UINT64_C(26),
               "required envelope bound changed");

int main(void) {
  uint8_t encoded[ENVELOPE_MAX_ENCODED_SIZE];
  size_t length = 99U;
  envelope_t value = {0};
  envelope_t decoded = {0};

  envelope_clear(&value);
  if (value.sequence != 0U || value.child.delta != 0 || value.child.axes[0] != 0U) return 1;
  if (envelope_encoded_size(&value) == SIZE_MAX) return 2;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) != WL_CODEC_OK) return 3;
  value.sequence = 150U;
  value.child.delta = -1;
  value.child.axes[0] = UINT32_C(0x01020304);
  value.child.axes[1] = UINT32_C(0xAABBCCDD);
  if (envelope_encoded_size(&value) != sizeof(golden)) return 7;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != sizeof(golden) || memcmp(encoded, golden, sizeof(golden)) != 0)
    return 8;

  if (envelope_decode(golden, sizeof(golden), &decoded) != WL_CODEC_OK ||
      decoded.sequence != 150U || decoded.child.delta != -1 ||
      decoded.child.axes[0] != UINT32_C(0x01020304) ||
      decoded.child.axes[1] != UINT32_C(0xAABBCCDD)) return 9;

  {
    uint8_t with_unknown[sizeof(golden) + 2U];
    memcpy(with_unknown, golden, sizeof(golden));
    with_unknown[sizeof(golden)] = 0x78U;
    with_unknown[sizeof(golden) + 1U] = 0x01U;
    if (envelope_decode(with_unknown, sizeof(with_unknown), &decoded) != WL_CODEC_OK)
      return 10;
  }
  {
    static const uint8_t unknown_only[] = {0x78U, 0x01U};
    if (envelope_decode(unknown_only, sizeof(unknown_only), &decoded) !=
        WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 11;
  }
  if (envelope_decode(NULL, 0U, &decoded) !=
      WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 12;
  {
    static const uint8_t sequence_only[] = {0x08U, 0x96U, 0x01U};
    if (envelope_decode(sequence_only, sizeof(sequence_only), &decoded) !=
        WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 13;
  }
  {
    static const uint8_t child_missing_axes[] = {
      0x08U, 0x96U, 0x01U, 0x12U, 0x02U, 0x08U, 0x01U
    };
    if (envelope_decode(child_missing_axes, sizeof(child_missing_axes), &decoded) !=
        WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 14;
  }
  {
    static const uint8_t child_missing_delta[] = {
      0x08U, 0x96U, 0x01U, 0x12U, 0x0AU,
      0x12U, 0x08U, 0U, 0U, 0U, 0U, 0U, 0U, 0U, 0U
    };
    if (envelope_decode(child_missing_delta, sizeof(child_missing_delta), &decoded) !=
        WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 15;
  }
  {
    static const uint8_t duplicate[] = {0x08U, 0x01U, 0x08U, 0x02U};
    if (envelope_decode(duplicate, sizeof(duplicate), &decoded) !=
        WL_CODEC_ERR_DUPLICATE_FIELD) return 16;
  }
  {
    static const uint8_t truncated_scalar[] = {0x08U};
    static const uint8_t truncated_nested[] = {
      0x08U, 0x01U, 0x12U, 0x02U, 0x08U
    };
    if (envelope_decode(truncated_scalar, sizeof(truncated_scalar), &decoded) !=
        WL_CODEC_ERR_MALFORMED) return 17;
    if (envelope_decode(truncated_nested, sizeof(truncated_nested), &decoded) !=
        WL_CODEC_ERR_MALFORMED) return 18;
  }
  {
    packed_t packed = {{0}}, copy;
    uint8_t wire[32]; size_t n = 0U;
    if (packed_encode(&packed, wire, sizeof(wire), &n) != WL_CODEC_OK || n != 10U) return 19;
    if (packed_decode(wire, n, &copy) != WL_CODEC_OK || copy.axes[1] != 0U) return 20;
    if (packed_decode(NULL, 0U, &copy) != WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 21;
    memcpy(wire + n, wire, n);
    if (packed_decode(wire, n * 2U, &copy) != WL_CODEC_ERR_DUPLICATE_FIELD) return 22;
  }
  return 0;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("required-fields-test");
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
        .arg(wirelink_include().parent().unwrap().join("src/codec.c"))
        .arg(directory.path().join("required_fields.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "required codec must be strict C11-clean");
    assert!(Command::new(executable).status().unwrap().success());

    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "required_fields.h"

#include <type_traits>

static_assert(std::is_same_v<decltype(envelope_t::sequence), uint32_t>);
static_assert(std::is_same_v<decltype(child_t::axes), uint32_t[2]>);
static_assert(sizeof(child_t) == 3U * sizeof(uint32_t));
static_assert(ENVELOPE_MAX_ENCODED_SIZE == UINT64_C(26));

int main() {
  envelope_t value{};
  return value.sequence != 0U;
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
    assert!(cxx_status.success(), "required header must be C++20-clean");
}
