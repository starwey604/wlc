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
fn generated_required_fields_enforce_presence_in_c11_and_compile_as_cxx20() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
message Child = 1 {
  required int32 delta = 1;
  required packed fixed32 axes[2] = 2;
}
message Envelope = 2 {
  required uint32 sequence = 1;
  required Child child = 2;
  optional bool enabled = 3;
}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "required_fields").unwrap();
    assert!(generated.header.contains("bool has_sequence;"));
    assert!(generated.header.contains("bool has_child;"));
    assert!(generated.header.contains("bool has_axes;"));
    assert!(
        generated
            .source
            .contains("WL_CODEC_ERR_MISSING_REQUIRED_FIELD")
    );

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
      0x01U, 0x02U, 0x03U, 0x04U,
      0xAAU, 0xBBU, 0xCCU, 0xDDU
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
  if (value.has_sequence || value.has_child || value.child.has_delta ||
      value.child.has_axes) return 1;
  if (envelope_encoded_size(&value) != SIZE_MAX) return 2;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) !=
          WL_CODEC_ERR_MISSING_REQUIRED_FIELD ||
      length != 99U) return 3;

  value.has_sequence = true;
  value.sequence = 150U;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 4;
  value.has_child = true;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 5;
  value.child.has_delta = true;
  value.child.delta = -1;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) !=
      WL_CODEC_ERR_MISSING_REQUIRED_FIELD) return 6;
  value.child.has_axes = true;
  value.child.axes[0] = UINT32_C(0x01020304);
  value.child.axes[1] = UINT32_C(0xAABBCCDD);
  if (envelope_encoded_size(&value) != sizeof(golden)) return 7;
  if (envelope_encode(&value, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != sizeof(golden) || memcmp(encoded, golden, sizeof(golden)) != 0)
    return 8;

  if (envelope_decode(golden, sizeof(golden), &decoded) != WL_CODEC_OK ||
      !decoded.has_sequence || decoded.sequence != 150U || !decoded.has_child ||
      !decoded.child.has_delta || decoded.child.delta != -1 ||
      !decoded.child.has_axes ||
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

static_assert(std::is_same_v<decltype(envelope_t::has_sequence), bool>);
static_assert(std::is_same_v<decltype(child_t::has_axes), bool>);
static_assert(ENVELOPE_MAX_ENCODED_SIZE == UINT64_C(26));

int main() {
  envelope_t value{};
  return value.has_sequence ? 1 : 0;
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
