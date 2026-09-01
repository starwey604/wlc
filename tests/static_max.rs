use std::{fs, path::Path, process::Command};

use proptest::prelude::*;
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

fn wirelink_root() -> std::path::PathBuf {
    fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("include"),
    )
    .unwrap()
    .parent()
    .unwrap()
    .to_path_buf()
}

fn macro_value(header: &str, name: &str) -> Option<u64> {
    let prefix = format!("#define {name} UINT64_C(");
    header
        .lines()
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix(')')?.parse().ok())
}

fn varint_size(mut value: u64) -> u64 {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}

#[test]
fn generated_static_max_is_exact_and_propagates_unbounded_storage() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
enum Mode = 1 { OFF = 0; ON = 1; }
message Leaf = 2 {
  optional bool boolean = 1;
  optional uint32 unsigned32 = 2;
  optional uint64 unsigned64 = 3;
  optional int32 signed32 = 4;
  optional int64 signed64 = 5;
  optional fixed32 fixed_32 = 6;
  optional fixed64 fixed_64 = 7;
  optional float32 float_32 = 8;
  optional float64 float_64 = 9;
  optional Mode mode = 10;
  packed fixed32 packed_32[3] = 16;
  packed float64 packed_64[16] = 2048;
}
message Outer = 3 {
  optional Leaf child = 1;
  packed float32 axes[30] = 16;
  optional uint32 tail = 65535;
}
message Empty = 4 {}
message Text = 5 { optional string value = 1; }
message Blob = 6 { optional bytes value = 1; }
message Series = 7 { repeated fixed32 values = 1; }
message TextEnvelope = 8 { optional Text child = 1; }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "bounded").unwrap();

    assert_eq!(
        macro_value(&generated.header, "LEAF_MAX_ENCODED_SIZE"),
        Some(218)
    );
    assert_eq!(
        macro_value(&generated.header, "OUTER_MAX_ENCODED_SIZE"),
        Some(352)
    );
    assert_eq!(
        macro_value(&generated.header, "EMPTY_MAX_ENCODED_SIZE"),
        Some(0)
    );
    for name in ["TEXT", "BLOB", "SERIES", "TEXT_ENVELOPE"] {
        assert!(
            generated
                .header
                .contains(&format!("#define {name}_HAS_MAX_ENCODED_SIZE 0"))
        );
        assert_eq!(
            macro_value(&generated.header, &format!("{name}_MAX_ENCODED_SIZE")),
            None
        );
    }

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("bounded.h"), generated.header).unwrap();
    fs::write(directory.path().join("bounded.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "bounded.h"

#include <limits.h>
#include <stdint.h>

#if !LEAF_HAS_MAX_ENCODED_SIZE || !OUTER_HAS_MAX_ENCODED_SIZE || \
    !EMPTY_HAS_MAX_ENCODED_SIZE
#error "bounded messages must advertise a static maximum"
#endif
#if TEXT_HAS_MAX_ENCODED_SIZE || BLOB_HAS_MAX_ENCODED_SIZE || \
    SERIES_HAS_MAX_ENCODED_SIZE || TEXT_ENVELOPE_HAS_MAX_ENCODED_SIZE
#error "dynamic storage must not advertise a static maximum"
#endif
#if defined(TEXT_MAX_ENCODED_SIZE) || defined(BLOB_MAX_ENCODED_SIZE) || \
    defined(SERIES_MAX_ENCODED_SIZE) || defined(TEXT_ENVELOPE_MAX_ENCODED_SIZE)
#error "an unavailable maximum must not be emitted"
#endif

_Static_assert(LEAF_MAX_ENCODED_SIZE == UINT64_C(218), "leaf golden maximum");
_Static_assert(OUTER_MAX_ENCODED_SIZE == UINT64_C(352), "outer golden maximum");
_Static_assert(EMPTY_MAX_ENCODED_SIZE == UINT64_C(0), "empty golden maximum");
_Static_assert(OUTER_MAX_ENCODED_SIZE <= SIZE_MAX,
               "scratch bound must fit this target's size_t");

int main(void) {
  uint8_t encoded[OUTER_MAX_ENCODED_SIZE];
  size_t length = 0U;
  leaf_t leaf = {0};
  outer_t outer = {0};

  leaf.has_boolean = true;
  leaf.boolean = true;
  leaf.has_unsigned32 = true;
  leaf.unsigned32 = UINT32_MAX;
  leaf.has_unsigned64 = true;
  leaf.unsigned64 = UINT64_MAX;
  leaf.has_signed32 = true;
  leaf.signed32 = INT32_MIN;
  leaf.has_signed64 = true;
  leaf.signed64 = INT64_MIN;
  leaf.has_fixed_32 = true;
  leaf.fixed_32 = UINT32_MAX;
  leaf.has_fixed_64 = true;
  leaf.fixed_64 = UINT64_MAX;
  leaf.has_float_32 = true;
  leaf.has_float_64 = true;
  leaf.has_mode = true;
  leaf.mode = INT32_MIN;
  leaf.has_packed_32 = true;
  leaf.has_packed_64 = true;
  if (leaf_encoded_size(&leaf) != LEAF_MAX_ENCODED_SIZE) return 1;
  if (leaf_encode(&leaf, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != LEAF_MAX_ENCODED_SIZE) return 2;

  outer.has_child = true;
  outer.child = leaf;
  outer.has_axes = true;
  outer.has_tail = true;
  outer.tail = UINT32_MAX;
  if (outer_encoded_size(&outer) != OUTER_MAX_ENCODED_SIZE) return 3;
  if (outer_encode(&outer, encoded, sizeof(encoded), &length) != WL_CODEC_OK ||
      length != OUTER_MAX_ENCODED_SIZE) return 4;
  return 0;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("static-max-test");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(wirelink_root().join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("bounded.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "static maximum fixture must compile");
    assert!(Command::new(executable).status().unwrap().success());

    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "bounded.h"

#include <cstdint>

static_assert(OUTER_HAS_MAX_ENCODED_SIZE == 1);
static_assert(OUTER_MAX_ENCODED_SIZE == UINT64_C(352));
static_assert(OUTER_MAX_ENCODED_SIZE <= SIZE_MAX);

int main() {
  std::uint8_t scratch[static_cast<std::size_t>(OUTER_MAX_ENCODED_SIZE)]{};
  return sizeof(scratch) == OUTER_MAX_ENCODED_SIZE ? 0 : 1;
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
        .arg(wirelink_root().join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("header.cpp"))
        .status()
        .unwrap();
    assert!(
        cxx_status.success(),
        "static maximum macros must be C++20-clean"
    );
}

#[test]
fn generated_static_max_macros_reject_schema_macro_collisions() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
enum Names = 1 { VALUE_MAX_ENCODED_SIZE = 0; }
message Value = 2 { optional uint32 field = 1; }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let error = generate_c(&model, "collision").unwrap_err();
    assert!(
        error.0.contains("VALUE_MAX_ENCODED_SIZE"),
        "unexpected diagnostic: {error}"
    );
}

#[test]
fn fci_realtime_and_status_messages_have_compile_time_scratch_bounds() {
    let model = analyze_schema(
        &parse_schema(
            r#"version 1;
enum ArmMode = 20482 {
  ARM_MODE_PC = 0;
  ARM_MODE_DRAG = 1;
  ARM_MODE_DAMP = 2;
  ARM_MODE_RETRACTING = 3;
  ARM_MODE_TELEOP = 4;
}
message ArmStatus = 24577 {
  optional ArmMode mode = 1;
  optional fixed32 sequence = 2;
  optional fixed64 timestamp_us = 3;
  packed float32 joint_position[6] = 4;
  packed float32 joint_velocity[6] = 5;
  packed float32 joint_torque[6] = 6;
  packed float32 base_gravity[3] = 7;
  optional float32 gripper_position = 8;
  optional float32 gripper_velocity = 9;
  optional float32 gripper_torque = 10;
  packed float32 end_effector_transform[16] = 11;
  packed float32 external_wrench[6] = 12;
  optional fixed32 error_flags = 13;
  optional fixed64 last_sdk_timestamp_us = 14;
}
message JointMitCommand = 25345 {
  packed float32 position[6] = 1;
  packed float32 velocity[6] = 2;
  packed float32 torque[6] = 3;
  packed float32 kp[6] = 4;
  packed float32 kd[6] = 5;
  optional fixed32 dt_us = 6;
  optional fixed32 sequence = 7;
  optional bool gravity_compensation = 8;
  optional fixed64 sdk_timestamp_us = 9;
  optional fixed64 lease_token = 10;
}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let generated = generate_c(&model, "fci_arm").unwrap();
    assert_eq!(
        macro_value(&generated.header, "ARM_STATUS_MAX_ENCODED_SIZE"),
        Some(233)
    );
    assert_eq!(
        macro_value(&generated.header, "JOINT_MIT_COMMAND_MAX_ENCODED_SIZE"),
        Some(160)
    );

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("fci_arm.h"), generated.header).unwrap();
    fs::write(directory.path().join("fci_arm.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "fci_arm.h"

#include <limits.h>

_Static_assert(ARM_STATUS_MAX_ENCODED_SIZE == UINT64_C(233),
               "FCI status scratch bound changed");
_Static_assert(JOINT_MIT_COMMAND_MAX_ENCODED_SIZE == UINT64_C(160),
               "FCI realtime scratch bound changed");

int main(void) {
  uint8_t status_scratch[ARM_STATUS_MAX_ENCODED_SIZE];
  uint8_t command_scratch[JOINT_MIT_COMMAND_MAX_ENCODED_SIZE];
  arm_status_t status = {0};
  joint_mit_command_t command = {0};
  size_t length = 0U;

  status.has_mode = true;
  status.mode = INT32_MIN;
  status.has_sequence = true;
  status.has_timestamp_us = true;
  status.has_joint_position = true;
  status.has_joint_velocity = true;
  status.has_joint_torque = true;
  status.has_base_gravity = true;
  status.has_gripper_position = true;
  status.has_gripper_velocity = true;
  status.has_gripper_torque = true;
  status.has_end_effector_transform = true;
  status.has_external_wrench = true;
  status.has_error_flags = true;
  status.has_last_sdk_timestamp_us = true;
  if (arm_status_encode(&status, status_scratch, sizeof(status_scratch),
                        &length) != WL_CODEC_OK ||
      length != ARM_STATUS_MAX_ENCODED_SIZE) return 1;

  command.has_position = true;
  command.has_velocity = true;
  command.has_torque = true;
  command.has_kp = true;
  command.has_kd = true;
  command.has_dt_us = true;
  command.has_sequence = true;
  command.has_gravity_compensation = true;
  command.gravity_compensation = true;
  command.has_sdk_timestamp_us = true;
  command.has_lease_token = true;
  if (joint_mit_command_encode(&command, command_scratch,
                               sizeof(command_scratch), &length) != WL_CODEC_OK ||
      length != JOINT_MIT_COMMAND_MAX_ENCODED_SIZE) return 2;
  return 0;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("fci-static-max-test");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(wirelink_root().join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("fci_arm.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "FCI scratch-bound fixture must compile");
    assert!(Command::new(executable).status().unwrap().success());
}

proptest! {
    #[test]
    fn generated_nested_and_packed_max_matches_independent_formula(
        scalar_number in 1_u16..=u16::MAX,
        packed_count in 1_u16..=u16::MAX,
        child_number in 1_u16..=u16::MAX,
    ) {
        let packed_number = if scalar_number == u16::MAX {
            1
        } else {
            scalar_number + 1
        };
        let source = format!(
            "version 1; message Leaf = 1 {{ optional uint64 scalar = {scalar_number}; packed fixed64 values[{packed_count}] = {packed_number}; }} message Outer = 2 {{ optional Leaf child = {child_number}; }}"
        );
        let model = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
        let generated = generate_c(&model, "property").unwrap();

        let scalar_tag = varint_size(u64::from(scalar_number) << 3);
        let packed_payload = u64::from(packed_count) * 8;
        let packed_tag = varint_size((u64::from(packed_number) << 3) | 2);
        let leaf_maximum = scalar_tag + 10 + packed_tag
            + varint_size(packed_payload) + packed_payload;
        let outer_maximum = varint_size((u64::from(child_number) << 3) | 2)
            + varint_size(leaf_maximum) + leaf_maximum;

        prop_assert_eq!(
            macro_value(&generated.header, "LEAF_MAX_ENCODED_SIZE"),
            Some(leaf_maximum)
        );
        prop_assert_eq!(
            macro_value(&generated.header, "OUTER_MAX_ENCODED_SIZE"),
            Some(outer_maximum)
        );
    }
}
