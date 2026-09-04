use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

const SCHEMA: &str = r#"
version 1;
enum Status = 1 { OK = 0; FAILED = 1; }
message State = 2 { optional uint32 sequence = 1; }
message Alarm = 3 { optional uint32 code = 1; }
message Request = 4 {
  optional uint32 operation_id = 1;
  optional bytes body = 2;
}
message Response = 5 {
  optional uint32 operation_id = 1;
  optional Status status = 2;
}
"#;

const PROFILE: &str = r#"
profile version 1;
latest State { delivery = unreliable; }
fifo Alarm { delivery = reliable; }
rpc Execute {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
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
fn generated_runtime_assembly_validates_and_initializes_exact_storage() {
    let directory = tempdir().unwrap();
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "assembly").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "assembly").unwrap();

    for (name, contents) in [
        ("assembly.h", codec.header),
        ("assembly.c", codec.source),
        ("assembly_bindings.h", codec.bindings_header),
        ("assembly_bindings.c", codec.bindings_source),
        ("assembly_runtime.h", runtime.header),
        ("assembly_runtime.c", runtime.source),
    ] {
        fs::write(directory.path().join(name), contents).unwrap();
    }
    fs::write(
        directory.path().join("main.c"),
        r#"#include "assembly_runtime.h"

#include <stdint.h>
#include <string.h>

void wl_event_release(wl_ctx_t *ctx, const wl_event_t *event) {
  (void)ctx;
  (void)event;
}

int wl_send_unreliable(wl_ctx_t *ctx, uint16_t message_id,
                       const uint8_t *payload, size_t payload_len) {
  (void)ctx;
  (void)message_id;
  (void)payload;
  (void)payload_len;
  return WL_OK;
}

int wl_send_reliable(wl_ctx_t *ctx, uint16_t message_id,
                     const uint8_t *payload, size_t payload_len,
                     wl_tx_handle_t *out_handle) {
  (void)ctx;
  (void)message_id;
  (void)payload;
  (void)payload_len;
  if (out_handle != NULL) *out_handle = 1U;
  return WL_OK;
}

int wl_tx_payload_claim(wl_ctx_t *ctx, uint16_t message_id,
                        wl_delivery_t delivery,
                        wl_tx_payload_claim_t *out_claim) {
  (void)ctx;
  (void)message_id;
  (void)delivery;
  (void)out_claim;
  return WL_ERR_NOT_SUPPORTED;
}

int wl_tx_payload_commit(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim,
                         size_t payload_len, wl_tx_handle_t *out_handle) {
  (void)ctx;
  (void)claim;
  (void)payload_len;
  (void)out_handle;
  return WL_ERR_NOT_SUPPORTED;
}

int wl_tx_payload_abort(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim) {
  (void)ctx;
  (void)claim;
  return WL_ERR_NOT_SUPPORTED;
}

int wl_tx_take(wl_ctx_t *ctx, wl_tx_handle_t handle,
               wl_tx_result_t *out_result) {
  (void)ctx;
  (void)handle;
  if (out_result != NULL) memset(out_result, 0, sizeof(*out_result));
  return WL_OK;
}

static int32_t execute(void *user_data, const request_t *request,
                       const wl_rpc_server_request_t *server_request,
                       wl_delivery_t delivery) {
  (void)request;
  (void)server_request;
  (void)delivery;
  return user_data == (void *)(uintptr_t)0x1234U ? 0 : -1;
}

static assembly_runtime_config_t valid_config(void) {
  assembly_runtime_config_t config = {0};
  config.state_latest_initial_generation = 41U;
  config.alarm_fifo_capacity = 3U;
  config.rpc_client_enabled = 1U;
  config.rpc_client_slot_count = 2U;
  config.rpc_client_response_capacity = 64U;
  config.rpc_client_next_operation_id = 7U;
  config.rpc_server_enabled = 1U;
  config.rpc_server_pending_slot_count = 3U;
  config.rpc_server_cache_slot_count = 2U;
  config.rpc_server_response_capacity = 64U;
  config.rpc_server_pending_timeout_ms = 100U;
  config.rpc_server_cache_ttl_ms = 200U;
  config.rpc_server_cache_policy = WL_RPC_CACHE_REJECT_NEW;
  config.execute_canonical_request_capacity = 96U;
  config.execute_request_handler = execute;
  config.execute_user_data = (void *)(uintptr_t)0x1234U;
  return config;
}

static int unchanged(const void *value, const void *copy, size_t size) {
  return memcmp(value, copy, size) == 0;
}

int main(void) {
  union {
    max_align_t align;
    uint8_t bytes[8192];
  } arena;
  assembly_runtime_instance_t instance;
  assembly_runtime_instance_t before;
  assembly_runtime_requirements_t requirements = {0};
  assembly_runtime_config_t config = valid_config();
  assembly_runtime_config_t defaults;
  assembly_runtime_storage_t storage;
  wl_latest_stats_t latest_stats = {0};
  wl_fifo_stats_t fifo_stats = {0};
  uint32_t operation_id = 0U;
  int result;

  if (ASSEMBLY_RUNTIME_HAS_DEFAULT_STORAGE != 0 ||
      assembly_runtime_config_defaults(NULL) != WL_ERR_INVALID_ARG ||
      assembly_runtime_config_defaults(&defaults) != WL_OK ||
      defaults.state_latest_initial_generation != 1U ||
      defaults.alarm_fifo_capacity != 1U ||
      defaults.rpc_client_enabled != 0U ||
      defaults.rpc_server_enabled != 0U ||
      defaults.rpc_client_response_capacity == 0U ||
      defaults.execute_canonical_request_capacity != 0U ||
      assembly_runtime_config_enable_client(&defaults) != WL_OK ||
      defaults.rpc_client_enabled != 1U ||
      assembly_runtime_config_enable_server(&defaults) !=
          WL_ERR_NOT_SUPPORTED)
    return 30;
  defaults.execute_canonical_request_capacity = 64U;
  if (assembly_runtime_config_enable_server(&defaults) != WL_OK ||
      defaults.rpc_server_enabled != 1U)
    return 31;

  result = assembly_runtime_requirements(&config, &requirements);
  if (result != WL_OK || requirements.storage_size == 0U ||
      requirements.storage_size > sizeof(arena.bytes) ||
      requirements.storage_alignment == 0U ||
      (requirements.storage_alignment &
       (requirements.storage_alignment - 1U)) != 0U ||
      ((uintptr_t)arena.bytes & (requirements.storage_alignment - 1U)) != 0U)
    return 1;

  memset(&instance, 0xA5, sizeof(instance));
  before = instance;
  memset(arena.bytes, 0x5A, sizeof(arena.bytes));
  storage = (assembly_runtime_storage_t){arena.bytes,
                                         requirements.storage_size - 1U};
  if (assembly_runtime_init(&instance, &config, &storage) !=
          WL_ERR_BUF_TOO_SMALL ||
      !unchanged(&instance, &before, sizeof(instance)))
    return 2;

  storage = (assembly_runtime_storage_t){arena.bytes + 1U,
                                         requirements.storage_size};
  if (assembly_runtime_init(&instance, &config, &storage) !=
          WL_ERR_INVALID_ARG ||
      !unchanged(&instance, &before, sizeof(instance)))
    return 3;

  storage = (assembly_runtime_storage_t){&instance, requirements.storage_size};
  if (assembly_runtime_init(&instance, &config, &storage) !=
          WL_ERR_INVALID_ARG ||
      !unchanged(&instance, &before, sizeof(instance)))
    return 4;

  storage = (assembly_runtime_storage_t){arena.bytes,
                                         requirements.storage_size};
  if (assembly_runtime_init(&instance, &config, &storage) != WL_OK) return 5;
  if (instance.runtime.state_latest != &instance.state_latest ||
      instance.runtime.alarm_fifo != &instance.alarm_fifo ||
      instance.runtime.rpc_client != &instance.rpc_client ||
      instance.runtime.rpc_server != &instance.rpc_server ||
      instance.runtime.execute.request_scratch !=
          &instance.execute_scratch.request ||
      instance.runtime.execute.response_scratch !=
          &instance.execute_scratch.response ||
      instance.runtime.execute.request_handler != execute ||
      instance.runtime.execute.user_data != (void *)(uintptr_t)0x1234U ||
      instance.runtime.execute.canonical_request_scratch.capacity != 96U)
    return 6;
  if ((uintptr_t)instance.runtime.execute.canonical_request_scratch.data <
          (uintptr_t)arena.bytes ||
      (uintptr_t)instance.runtime.execute.canonical_request_scratch.data + 96U >
          (uintptr_t)arena.bytes + requirements.storage_size)
    return 7;
  if (wl_latest_get_stats(&instance.state_latest, &latest_stats) != WL_OK ||
      latest_stats.generation != 41U ||
      wl_fifo_get_stats(&instance.alarm_fifo, &fifo_stats) != WL_OK ||
      fifo_stats.depth != 0U)
    return 8;
  if (wl_rpc_client_begin(&instance.rpc_client, REQUEST_MESSAGE_ID,
                          RESPONSE_MESSAGE_ID, 10U, 0U,
                          &operation_id) != WL_RPC_OK ||
      operation_id != 7U)
    return 9;

  config.alarm_fifo_capacity = 0U;
  requirements = (assembly_runtime_requirements_t){99U, 99U};
  if (assembly_runtime_requirements(&config, &requirements) !=
          WL_ERR_INVALID_ARG ||
      requirements.storage_size != 0U || requirements.storage_alignment != 0U)
    return 10;
  config = valid_config();
  config.rpc_client_slot_count = 0U;
  if (assembly_runtime_requirements(&config, &requirements) !=
      WL_ERR_INVALID_ARG)
    return 11;
  config = valid_config();
  config.rpc_server_cache_slot_count = 0U;
  if (assembly_runtime_requirements(&config, &requirements) !=
      WL_ERR_INVALID_ARG)
    return 12;
  config = valid_config();
  config.execute_canonical_request_capacity = 0U;
  if (assembly_runtime_requirements(&config, &requirements) !=
      WL_ERR_INVALID_ARG)
    return 13;
  config = valid_config();
  config.rpc_client_enabled = 2U;
  if (assembly_runtime_requirements(&config, &requirements) !=
      WL_ERR_INVALID_ARG)
    return 14;
  config = valid_config();
  config.execute_canonical_request_capacity = SIZE_MAX;
  if (assembly_runtime_requirements(&config, &requirements) !=
      WL_ERR_INVALID_ARG)
    return 15;
  memset(&instance, 0xA5, sizeof(instance));
  before = instance;
  storage = (assembly_runtime_storage_t){arena.bytes, sizeof(arena.bytes)};
  if (assembly_runtime_init(&instance, &config, &storage) !=
          WL_ERR_INVALID_ARG ||
      !unchanged(&instance, &before, sizeof(instance)))
    return 16;

  config = valid_config();
  config.rpc_client_enabled = 0U;
  config.rpc_client_slot_count = 0U;
  config.rpc_client_response_capacity = 0U;
  config.rpc_server_enabled = 0U;
  config.rpc_server_pending_slot_count = 0U;
  config.rpc_server_cache_slot_count = 0U;
  config.rpc_server_response_capacity = 0U;
  config.execute_canonical_request_capacity = 0U;
  if (assembly_runtime_requirements(&config, &requirements) != WL_OK ||
      requirements.storage_size == 0U)
    return 17;
  storage = (assembly_runtime_storage_t){arena.bytes,
                                         requirements.storage_size};
  if (assembly_runtime_init(&instance, &config, &storage) != WL_OK ||
      instance.runtime.state_latest == NULL ||
      instance.runtime.alarm_fifo == NULL ||
      instance.runtime.rpc_client != NULL ||
      instance.runtime.rpc_server != NULL ||
      instance.runtime.execute.request_scratch != NULL ||
      instance.runtime.execute.response_scratch != NULL)
    return 18;
  return 0;
}
"#,
    )
    .unwrap();

    let root = wirelink_root();
    let executable = directory.path().join("runtime-assembly-test");
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
        .arg(directory.path().join("assembly.c"))
        .arg(directory.path().join("assembly_bindings.c"))
        .arg(directory.path().join("assembly_runtime.c"))
        .arg(root.join("src/latest.c"))
        .arg(root.join("src/fifo.c"))
        .arg(root.join("src/rpc.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "generated assembly must compile cleanly");
    let run = Command::new(&executable).status().unwrap();
    assert!(run.success(), "generated assembly exited with {run}");
}

#[test]
fn generated_runtime_assembly_clears_instance_after_component_init_failure() {
    let directory = tempdir().unwrap();
    let schema = analyze_schema(
        &parse_schema("version 1; message State = 1 { optional uint32 sequence = 1; }").unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile("profile version 1; latest State { delivery = unreliable; }")
            .unwrap(),
        &schema,
    )
    .unwrap();
    let codec = generate_c(&schema, "rollback").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "rollback").unwrap();

    fs::write(directory.path().join("rollback.h"), codec.header).unwrap();
    fs::write(directory.path().join("rollback.c"), codec.source).unwrap();
    fs::write(
        directory.path().join("rollback_bindings.h"),
        codec.bindings_header,
    )
    .unwrap();
    fs::write(directory.path().join("rollback_runtime.h"), runtime.header).unwrap();
    fs::write(directory.path().join("rollback_runtime.c"), runtime.source).unwrap();
    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "rollback_runtime.h"

static_assert(ROLLBACK_RUNTIME_HAS_DEFAULT_STORAGE == 1);
static_assert(sizeof(rollback_runtime_default_storage_t) >=
              ROLLBACK_RUNTIME_DEFAULT_STORAGE_CAPACITY);
"#,
    )
    .unwrap();
    let header_status = Command::new("c++")
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
        header_status.success(),
        "generated bounded storage header must compile as C++20"
    );
    fs::write(
        directory.path().join("main.c"),
        r#"#include "rollback_runtime.h"

#include <stdint.h>
#include <string.h>

void wl_event_release(wl_ctx_t *ctx, const wl_event_t *event) {
  (void)ctx;
  (void)event;
}

int wl_latest_requirements(const wl_latest_config_t *config,
                           wl_latest_requirements_t *out_requirements) {
  if (config == NULL || out_requirements == NULL || config->value_size == 0U ||
      config->value_alignment == 0U)
    return WL_ERR_INVALID_ARG;
  out_requirements->storage_size = 256U;
  out_requirements->slot_stride = 64U;
  out_requirements->slot_count = WL_LATEST_SLOT_COUNT;
  return WL_OK;
}

int wl_latest_init(wl_latest_t *mailbox, const wl_latest_config_t *config,
                   const wl_latest_storage_t *storage) {
  (void)config;
  (void)storage;
  memset(mailbox, 0x3C, sizeof(*mailbox));
  return WL_ERR_NOT_SUPPORTED;
}

int wl_latest_write_claim(wl_latest_t *mailbox,
                          wl_latest_write_claim_t *out_claim) {
  (void)mailbox;
  (void)out_claim;
  return WL_ERR_NOT_INITIALIZED;
}

int wl_latest_write_publish(wl_latest_t *mailbox,
                            const wl_latest_write_claim_t *claim) {
  (void)mailbox;
  (void)claim;
  return WL_ERR_NOT_INITIALIZED;
}

int wl_latest_write_abort(wl_latest_t *mailbox,
                          const wl_latest_write_claim_t *claim) {
  (void)mailbox;
  (void)claim;
  return WL_ERR_NOT_INITIALIZED;
}

int wl_latest_read_acquire(wl_latest_t *mailbox,
                           wl_latest_view_t *out_view) {
  (void)mailbox;
  (void)out_view;
  return WL_ERR_NOT_INITIALIZED;
}

int wl_latest_read_release(wl_latest_t *mailbox,
                           const wl_latest_view_t *view) {
  (void)mailbox;
  (void)view;
  return WL_ERR_NOT_INITIALIZED;
}

int main(void) {
  union {
    max_align_t align;
    uint8_t bytes[256];
  } storage_bytes;
  rollback_runtime_config_t config = {0};
  rollback_runtime_requirements_t requirements = {0};
  rollback_runtime_instance_t instance;
  rollback_runtime_storage_t storage;
  const uint8_t *instance_bytes = (const uint8_t *)(const void *)&instance;
  size_t index;

  memset(&instance, 0xA5, sizeof(instance));
  if (rollback_runtime_requirements(&config, &requirements) != WL_OK ||
      requirements.storage_size != sizeof(storage_bytes.bytes))
    return 1;
  storage = (rollback_runtime_storage_t){storage_bytes.bytes,
                                         requirements.storage_size};
  if (rollback_runtime_init(&instance, &config, &storage) !=
      WL_ERR_NOT_SUPPORTED)
    return 2;
  for (index = 0U; index < sizeof(instance); ++index) {
    if (instance_bytes[index] != 0U) return 3;
  }
  return 0;
}
"#,
    )
    .unwrap();

    let root = wirelink_root();
    let executable = directory.path().join("runtime-rollback-test");
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
        .arg(directory.path().join("rollback.c"))
        .arg(directory.path().join("rollback_runtime.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "rollback fixture must compile cleanly");
    let run = Command::new(&executable).status().unwrap();
    assert!(run.success(), "rollback fixture exited with {run}");
}
