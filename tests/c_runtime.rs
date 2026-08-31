use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

const SCHEMA: &str = r#"
version 1;
message LatestValue = 1 {
  optional uint32 sequence = 1;
  packed float32 axes[2] = 2;
}
message Alarm = 2 {
  optional uint32 code = 1;
}
"#;

const PROFILE: &str = r#"
profile version 1;
latest LatestValue { delivery = unreliable; }
fifo Alarm { delivery = reliable; }
"#;

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

#[test]
fn generated_latest_and_fifo_runtime_compiles_and_releases_every_rx_once() {
    let directory = tempdir().unwrap();
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "typed_runtime").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "typed_runtime").unwrap();

    assert!(runtime.header.contains("TYPED_RUNTIME_SCHEMA_IDENTITY"));
    assert!(
        runtime
            .header
            .contains("TYPED_RUNTIME_BINDING_PROFILE_IDENTITY")
    );
    assert!(runtime.header.contains("wl_latest_t *latest_value_latest;"));
    assert!(runtime.header.contains("wl_fifo_t *alarm_fifo;"));
    assert!(runtime.source.contains("wl_latest_write_claim"));
    assert!(runtime.source.contains("wl_fifo_write_claim"));

    fs::write(directory.path().join("typed_runtime.h"), codec.header).unwrap();
    fs::write(directory.path().join("typed_runtime.c"), codec.source).unwrap();
    fs::write(
        directory.path().join("typed_runtime_bindings.h"),
        codec.bindings_header,
    )
    .unwrap();
    fs::write(
        directory.path().join("typed_runtime_runtime.h"),
        runtime.header,
    )
    .unwrap();
    fs::write(
        directory.path().join("typed_runtime_runtime.c"),
        runtime.source,
    )
    .unwrap();
    fs::write(
        directory.path().join("main.c"),
        r#"#include "typed_runtime_runtime.h"

#include <stdint.h>
#include <string.h>

static uint32_t releases;

void wl_event_release(wl_ctx_t *ctx, const wl_event_t *event) {
  (void)ctx;
  (void)event;
  ++releases;
}

static int dispatch_latest(wl_ctx_t *ctx, typed_runtime_runtime_t *runtime,
                           uint32_t sequence) {
  latest_value_t value = {0};
  uint8_t payload[32];
  size_t length = 0U;
  wl_event_t event = {0};
  typed_runtime_runtime_result_t result;

  value.has_sequence = true;
  value.sequence = sequence;
  value.has_axes = true;
  value.axes[0] = (float)sequence;
  value.axes[1] = -(float)sequence;
  if (latest_value_encode(&value, payload, sizeof(payload), &length) != WL_CODEC_OK)
    return 1;
  event.type = WL_EVT_UNRELIABLE_RX;
  event.message_id = LATEST_VALUE_MESSAGE_ID;
  event.payload = payload;
  event.payload_len = length;
  result = typed_runtime_runtime_dispatch_event(ctx, &event, runtime, 0U);
  if (result.domain == TYPED_RUNTIME_RUNTIME_OK) return 0;
  if (result.domain == TYPED_RUNTIME_RUNTIME_STORAGE_ERROR)
    return result.storage_result;
  return 2;
}

static int dispatch_alarm(wl_ctx_t *ctx, typed_runtime_runtime_t *runtime,
                          uint32_t code) {
  alarm_t value = {0};
  uint8_t payload[16];
  size_t length = 0U;
  wl_event_t event = {0};
  typed_runtime_runtime_result_t result;

  value.has_code = true;
  value.code = code;
  if (alarm_encode(&value, payload, sizeof(payload), &length) != WL_CODEC_OK)
    return 1;
  event.type = WL_EVT_RELIABLE_RX;
  event.message_id = ALARM_MESSAGE_ID;
  event.payload = payload;
  event.payload_len = length;
  result = typed_runtime_runtime_dispatch_event(ctx, &event, runtime, 0U);
  return result.domain == TYPED_RUNTIME_RUNTIME_OK ? 0 : result.storage_result;
}

int main(void) {
  wl_ctx_t ctx = {0};
  wl_latest_t latest = {0};
  latest_value_t latest_slots[WL_LATEST_SLOT_COUNT] = {0};
  const wl_latest_config_t latest_config = {
    sizeof(latest_slots[0]), _Alignof(latest_value_t), 0U
  };
  const wl_latest_storage_t latest_storage = {
    latest_slots, sizeof(latest_slots)
  };
  wl_fifo_t fifo = {0};
  alarm_t fifo_slots[2] = {0};
  const wl_fifo_config_t fifo_config = {
    sizeof(fifo_slots[0]), _Alignof(alarm_t), 2U
  };
  const wl_fifo_storage_t fifo_storage = { fifo_slots, sizeof(fifo_slots) };
  typed_runtime_runtime_t runtime = {0};
  wl_latest_view_t latest_view = {0};
  wl_fifo_view_t fifo_view = {0};
  wl_event_t event = {0};
  typed_runtime_runtime_result_t result;
  const uint8_t malformed[] = {0x80U};

  if (wl_latest_init(&latest, &latest_config, &latest_storage) != WL_OK ||
      wl_fifo_init(&fifo, &fifo_config, &fifo_storage) != WL_OK) return 1;
  runtime.latest_value_latest = &latest;
  runtime.alarm_fifo = &fifo;

  if (dispatch_latest(&ctx, &runtime, 7U) != 0 || releases != 1U) return 2;
  if (wl_latest_read_acquire(&latest, &latest_view) != WL_OK) return 3;
  if (((const latest_value_t *)latest_view.value)->sequence != 7U) return 4;
  if (wl_latest_read_release(&latest, &latest_view) != WL_OK) return 5;

  event.type = WL_EVT_UNRELIABLE_RX;
  event.message_id = LATEST_VALUE_MESSAGE_ID;
  event.payload = malformed;
  event.payload_len = sizeof(malformed);
  result = typed_runtime_runtime_dispatch_event(&ctx, &event, &runtime, 0U);
  if (result.domain != TYPED_RUNTIME_RUNTIME_CODEC_ERROR ||
      result.codec_status != WL_CODEC_ERR_MALFORMED ||
      result.abort_result != WL_OK || releases != 2U) return 6;
  if (dispatch_latest(&ctx, &runtime, 8U) != 0 || releases != 3U) return 7;

  event.type = WL_EVT_RELIABLE_RX;
  result = typed_runtime_runtime_dispatch_event(&ctx, &event, &runtime, 0U);
  if (result.domain != TYPED_RUNTIME_RUNTIME_DELIVERY_MISMATCH ||
      releases != 4U) return 8;

  if (dispatch_alarm(&ctx, &runtime, 10U) != 0 ||
      dispatch_alarm(&ctx, &runtime, 11U) != 0 || releases != 6U) return 9;
  if (dispatch_alarm(&ctx, &runtime, 12U) != WL_ERR_QUEUE_FULL ||
      releases != 7U) return 10;
  if (wl_fifo_read_acquire(&fifo, &fifo_view) != WL_OK ||
      ((const alarm_t *)fifo_view.value)->code != 10U ||
      wl_fifo_read_release(&fifo, &fifo_view) != WL_OK) return 11;
  if (wl_fifo_read_acquire(&fifo, &fifo_view) != WL_OK ||
      ((const alarm_t *)fifo_view.value)->code != 11U ||
      wl_fifo_read_release(&fifo, &fifo_view) != WL_OK) return 12;

  event.type = WL_EVT_UNRELIABLE_RX;
  event.message_id = 999U;
  result = typed_runtime_runtime_dispatch_event(&ctx, &event, &runtime, 0U);
  if (result.domain != TYPED_RUNTIME_RUNTIME_UNKNOWN_MESSAGE ||
      releases != 8U) return 13;
  result = typed_runtime_runtime_dispatch_event(&ctx, &event, NULL, 0U);
  if (result.domain != TYPED_RUNTIME_RUNTIME_INVALID_ARGUMENT ||
      releases != 9U) return 14;
  event.type = WL_EVT_TX_SUCCESS;
  result = typed_runtime_runtime_dispatch_event(&ctx, &event, &runtime, 0U);
  if (result.domain != TYPED_RUNTIME_RUNTIME_NON_RX || releases != 9U) return 15;

  {
    wl_latest_t undersized = {0};
    uint32_t slots[WL_LATEST_SLOT_COUNT] = {0};
    const wl_latest_config_t config = {
      sizeof(slots[0]), _Alignof(uint32_t), 0U
    };
    const wl_latest_storage_t storage = { slots, sizeof(slots) };
    if (wl_latest_init(&undersized, &config, &storage) != WL_OK) return 16;
    runtime.latest_value_latest = &undersized;
    if (dispatch_latest(&ctx, &runtime, 9U) != WL_ERR_BUF_TOO_SMALL ||
        releases != 10U) return 17;
  }
  {
    wl_latest_t misaligned = {0};
    union {
      max_align_t align;
      uint8_t bytes[WL_LATEST_SLOT_COUNT * sizeof(latest_value_t) + 1U];
    } slots = {0};
    const wl_latest_config_t config = {
      sizeof(latest_value_t), 1U, 0U
    };
    const wl_latest_storage_t storage = {
      slots.bytes + 1U, sizeof(slots.bytes) - 1U
    };
    if (wl_latest_init(&misaligned, &config, &storage) != WL_OK) return 18;
    runtime.latest_value_latest = &misaligned;
    if (dispatch_latest(&ctx, &runtime, 10U) != WL_ERR_INVALID_ARG ||
        releases != 11U) return 19;
  }
  return 0;
}
"#,
    )
    .unwrap();

    let root = wirelink_root();
    let executable = directory.path().join("runtime-test");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("typed_runtime.c"))
        .arg(directory.path().join("typed_runtime_runtime.c"))
        .arg(root.join("src/latest.c"))
        .arg(root.join("src/fifo.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "generated runtime must compile cleanly");
    assert!(Command::new(executable).status().unwrap().success());

    let cxx = Command::new("c++")
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("typed_runtime_runtime.h"))
        .status()
        .unwrap();
    assert!(
        cxx.success(),
        "generated runtime header must be C++20-clean"
    );
}
