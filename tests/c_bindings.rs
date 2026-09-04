use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, parse_schema};

const BINDINGS_SCHEMA: &str = r#"
version 1;
message Empty = 1 {}
message Child = 2 { repeated uint32 values = 1; }
message Envelope = 3 {
  repeated uint32 samples = 1;
  optional bytes borrowed = 2;
  optional Child child = 3;
  repeated Child children = 4;
}
"#;

fn write_generated(directory: &Path) {
    let model = analyze_schema(&parse_schema(BINDINGS_SCHEMA).unwrap()).unwrap();
    let generated = generate_c(&model, "typed_api").unwrap();
    fs::write(directory.join("typed_api.h"), generated.header).unwrap();
    fs::write(directory.join("typed_api.c"), generated.source).unwrap();
    fs::write(
        directory.join("typed_api_bindings.h"),
        generated.bindings_header,
    )
    .unwrap();
    fs::write(
        directory.join("typed_api_bindings.c"),
        generated.bindings_source,
    )
    .unwrap();
}

fn wirelink_include() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("include")
}

#[test]
fn generated_bindings_dispatch_and_send_without_allocation() {
    let directory = tempdir().unwrap();
    write_generated(directory.path());
    fs::write(
        directory.path().join("main.c"),
        r#"#include "typed_api_bindings.h"

#include <stdint.h>
#include <string.h>

typedef struct {
  uint32_t *samples;
  uint32_t *child_values;
  child_t *children;
  const uint8_t *payload;
  wl_delivery_t expected_delivery;
  int32_t handler_result;
  uint32_t expected_first_sample;
  size_t expected_sample_count;
} handler_state_t;

static uint32_t release_calls;
static uint32_t send_calls;
static uint32_t send_reliable;
static uint16_t sent_message_id;
static uint8_t sent_payload[64];
static size_t sent_payload_length;
static int next_core_result = WL_OK;
static uint8_t direct_payload[64];
static size_t direct_capacity = sizeof(direct_payload);
static uint32_t direct_claim_active;
static uint32_t direct_abort_calls;
static uint32_t direct_supported = 1U;
static uint16_t direct_message_id;
static wl_delivery_t direct_delivery;

void wl_event_release(wl_ctx_t *ctx, const wl_event_t *event) {
  (void)ctx;
  (void)event;
  ++release_calls;
}

int wl_send_unreliable(wl_ctx_t *ctx, uint16_t message_id,
                       const uint8_t *payload, size_t payload_len) {
  (void)ctx;
  ++send_calls;
  send_reliable = 0U;
  sent_message_id = message_id;
  sent_payload_length = payload_len;
  if (payload_len != 0U) memcpy(sent_payload, payload, payload_len);
  return next_core_result;
}

int wl_send_reliable(wl_ctx_t *ctx, uint16_t message_id,
                     const uint8_t *payload, size_t payload_len,
                     wl_tx_handle_t *out_handle) {
  (void)ctx;
  ++send_calls;
  send_reliable = 1U;
  sent_message_id = message_id;
  sent_payload_length = payload_len;
  if (payload_len != 0U) memcpy(sent_payload, payload, payload_len);
  if (next_core_result == WL_OK) *out_handle = UINT32_C(0x12345678);
  return next_core_result;
}

int wl_tx_payload_claim(wl_ctx_t *ctx, uint16_t message_id,
                        wl_delivery_t delivery,
                        wl_tx_payload_claim_t *out_claim) {
  (void)ctx;
  if (direct_supported == 0U) return WL_ERR_NOT_SUPPORTED;
  if (direct_claim_active != 0U) return WL_ERR_BUSY;
  direct_claim_active = 1U;
  direct_message_id = message_id;
  direct_delivery = delivery;
  out_claim->span.data = direct_payload;
  out_claim->span.length = direct_capacity;
  out_claim->token = 91U;
  return WL_OK;
}

int wl_tx_payload_commit(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim,
                         size_t payload_len, wl_tx_handle_t *out_handle) {
  (void)ctx;
  if (direct_claim_active == 0U || claim->token != 91U) return WL_ERR_NOT_FOUND;
  direct_claim_active = 0U;
  sent_payload_length = payload_len;
  memcpy(sent_payload, claim->span.data, payload_len);
  if (out_handle != NULL) *out_handle = UINT32_C(0x87654321);
  return next_core_result;
}

int wl_tx_payload_abort(wl_ctx_t *ctx, const wl_tx_payload_claim_t *claim) {
  (void)ctx;
  ++direct_abort_calls;
  if (direct_claim_active == 0U || claim->token != 91U) return WL_ERR_NOT_FOUND;
  direct_claim_active = 0U;
  return WL_OK;
}

static int32_t handle_envelope(void *user_data, const envelope_t *message,
                               wl_delivery_t delivery) {
  handler_state_t *state = user_data;
  if (release_calls != 0U) return -1;
  if (delivery != state->expected_delivery) return -2;
  if (message->samples != state->samples ||
      message->samples_capacity != 4U ||
      message->samples_count != state->expected_sample_count ||
      message->samples[0] != state->expected_first_sample) return -3;
  if (state->expected_sample_count == 2U) {
    if (!message->has_borrowed || message->borrowed.length != 2U ||
        message->borrowed.data != state->payload + 6U)
      return -4;
    if (!message->has_child || message->child.values != state->child_values ||
        message->child.values_capacity != 4U ||
        message->child.values_count != 2U ||
        message->child.values[0] != 5U || message->child.values[1] != 6U)
      return -5;
    if (message->children != state->children ||
        message->children_capacity != 2U || message->children_count != 1U ||
        message->children[0].values_count != 1U ||
        message->children[0].values[0] != 7U)
      return -6;
  }
  return state->handler_result;
}

static int check_dispatch(wl_ctx_t *ctx) {
  static const uint8_t first_payload[] = {
    0x08U, 0x03U, 0x08U, 0x09U,
    0x12U, 0x02U, 0xAAU, 0xBBU,
    0x1AU, 0x04U, 0x08U, 0x05U, 0x08U, 0x06U,
    0x22U, 0x02U, 0x08U, 0x07U
  };
  static const uint8_t second_payload[] = { 0x08U, 0x0BU };
  static const uint8_t malformed[] = { 0x80U };
  uint32_t samples[4] = {0U};
  uint32_t child_values[4] = {0U};
  uint32_t first_child_values[2] = {0U};
  uint32_t second_child_values[2] = {0U};
  child_t children[2] = {0};
  envelope_t scratch = {0};
  handler_state_t state = {
    samples, child_values, children, first_payload, WL_DELIVERY_RELIABLE,
    0, 3U, 2U
  };
  typed_api_router_t router = {0};
  wl_event_t event = {0};
  typed_api_dispatch_result_t result;

  scratch.samples = samples;
  scratch.samples_capacity = 4U;
  scratch.child.values = child_values;
  scratch.child.values_capacity = 4U;
  scratch.children = children;
  scratch.children_capacity = 2U;
  children[0].values = first_child_values;
  children[0].values_capacity = 2U;
  children[1].values = second_child_values;
  children[1].values_capacity = 2U;
  router.envelope = (typed_api_envelope_route_t){ &scratch, handle_envelope, &state };
  event.type = WL_EVT_RELIABLE_RX;
  event.message_id = ENVELOPE_MESSAGE_ID;
  event.payload = first_payload;
  event.payload_len = sizeof(first_payload);

  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_OK || result.codec_status != WL_CODEC_OK ||
      result.handler_result != 0 || release_calls != 1U) return 1;
  if (scratch.samples != samples || scratch.child.values != child_values ||
      scratch.children != children || children[0].values != first_child_values)
    return 2;

  release_calls = 0U;
  state.payload = second_payload;
  state.expected_delivery = WL_DELIVERY_UNRELIABLE;
  state.expected_first_sample = 11U;
  state.expected_sample_count = 1U;
  event.type = WL_EVT_UNRELIABLE_RX;
  event.payload = second_payload;
  event.payload_len = sizeof(second_payload);
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_OK || release_calls != 1U ||
      scratch.samples != samples || scratch.samples_count != 1U ||
      scratch.has_child || scratch.children_count != 0U) return 3;

  event.type = WL_EVT_RELIABLE_RX;
  release_calls = 0U;
  event.message_id = 99U;
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_UNKNOWN_MESSAGE || release_calls != 1U)
    return 4;
  release_calls = 0U;
  result = typed_api_dispatch_event(ctx, &event, NULL);
  if (result.domain != TYPED_API_DISPATCH_UNKNOWN_MESSAGE || release_calls != 1U)
    return 5;

  event.message_id = ENVELOPE_MESSAGE_ID;
  release_calls = 0U;
  result = typed_api_dispatch_event(ctx, &event, NULL);
  if (result.domain != TYPED_API_DISPATCH_MISSING_ROUTE || release_calls != 1U)
    return 6;
  release_calls = 0U;
  router.envelope.handler = NULL;
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_MISSING_ROUTE || release_calls != 1U)
    return 7;
  release_calls = 0U;
  router.envelope.handler = handle_envelope;
  router.envelope.scratch = NULL;
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_MISSING_SCRATCH || release_calls != 1U)
    return 8;

  release_calls = 0U;
  router.envelope.scratch = &scratch;
  event.payload = malformed;
  event.payload_len = sizeof(malformed);
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_CODEC_ERROR ||
      result.codec_status != WL_CODEC_ERR_MALFORMED || release_calls != 1U)
    return 9;

  release_calls = 0U;
  event.payload = second_payload;
  event.payload_len = sizeof(second_payload);
  state.payload = second_payload;
  state.expected_delivery = WL_DELIVERY_RELIABLE;
  state.handler_result = -77;
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_HANDLER_ERROR ||
      result.handler_result != -77 || release_calls != 1U) return 10;

  release_calls = 0U;
  event.type = WL_EVT_TX_SUCCESS;
  result = typed_api_dispatch_event(ctx, &event, &router);
  if (result.domain != TYPED_API_DISPATCH_NON_RX || release_calls != 0U)
    return 11;
  if (router.counters.delivered != 2U || router.counters.non_rx != 1U ||
      router.counters.unknown_message != 1U || router.counters.missing_route != 1U ||
      router.counters.missing_scratch != 1U || router.counters.codec_failure != 1U ||
      router.counters.handler_failure != 1U) return 12;
  return 0;
}

static int check_send(wl_ctx_t *ctx) {
  uint32_t samples[] = {3U};
  envelope_t envelope = {0};
  empty_t empty = {0};
  typed_api_send_result_t result;
  envelope.samples = samples;
  envelope.samples_count = 1U;
  envelope.samples_capacity = 1U;

  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_UNRELIABLE);
  if (result.domain != TYPED_API_SEND_OK || result.codec_status != WL_CODEC_OK ||
      result.core_result != WL_OK ||
      result.payload_length != 2U || result.handle != 0U ||
      direct_message_id != ENVELOPE_MESSAGE_ID ||
      direct_delivery != WL_DELIVERY_UNRELIABLE ||
      sent_payload_length != 2U || sent_payload[0] != 0x08U || sent_payload[1] != 0x03U)
    return 1;

  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_RELIABLE);
  if (result.domain != TYPED_API_SEND_OK ||
      result.handle != UINT32_C(0x87654321) ||
      direct_delivery != WL_DELIVERY_RELIABLE)
    return 2;

  next_core_result = WL_ERR_BUSY;
  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_RELIABLE);
  if (result.domain != TYPED_API_SEND_CORE_ERROR ||
      result.core_result != WL_ERR_BUSY || direct_claim_active != 0U ||
      direct_abort_calls != 0U) return 3;

  next_core_result = WL_OK;
  direct_capacity = 1U;
  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_UNRELIABLE);
  if (result.domain != TYPED_API_SEND_CODEC_ERROR ||
      result.codec_status != WL_CODEC_ERR_CAPACITY || direct_abort_calls != 1U ||
      direct_claim_active != 0U) return 4;

  direct_capacity = sizeof(direct_payload);
  result = typed_api_empty_send(ctx, &empty, WL_DELIVERY_UNRELIABLE);
  if (result.domain != TYPED_API_SEND_OK || result.payload_length != 0U ||
      direct_message_id != EMPTY_MESSAGE_ID ||
      sent_payload_length != 0U) return 5;

  direct_supported = 0U;
  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_UNRELIABLE);
  if (result.domain != TYPED_API_SEND_CORE_ERROR ||
      result.core_result != WL_ERR_NOT_SUPPORTED ||
      direct_claim_active != 0U) return 6;

  direct_supported = 1U;
  result = typed_api_envelope_send(ctx, &envelope, WL_DELIVERY_UNRELIABLE);
  if (result.domain != TYPED_API_SEND_OK || direct_claim_active != 0U ||
      sent_payload_length != 2U) return 7;
  return 0;
}

int main(void) {
  wl_ctx_t ctx = {0};
  int result = check_dispatch(&ctx);
  if (result != 0) return result;
  result = check_send(&ctx);
  return result == 0 ? 0 : 100 + result;
}
"#,
    )
    .unwrap();

    let executable = directory.path().join("bindings-test");
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
        .arg(directory.path().join("typed_api.c"))
        .arg(directory.path().join("typed_api_bindings.c"))
        .arg(directory.path().join("main.c"))
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "generated bindings must compile cleanly");
    assert!(Command::new(executable).status().unwrap().success());
}

#[test]
fn generated_codec_stays_core_independent_and_bindings_header_is_cxx20() {
    let directory = tempdir().unwrap();
    write_generated(directory.path());
    fs::write(
        directory.path().join("codec_only.c"),
        r#"#include "typed_api.h"
int main(void) {
  empty_t value = {0};
  size_t length = 1U;
  return empty_encode(&value, NULL, 0U, &length) == WL_CODEC_OK && length == 0U ? 0 : 1;
}
"#,
    )
    .unwrap();
    let codec_executable = directory.path().join("codec-only");
    let codec_status = Command::new("cc")
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
        .arg(directory.path().join("typed_api.c"))
        .arg(directory.path().join("codec_only.c"))
        .arg("-o")
        .arg(&codec_executable)
        .status()
        .unwrap();
    assert!(
        codec_status.success(),
        "codec object must not require core symbols"
    );
    assert!(Command::new(codec_executable).status().unwrap().success());

    fs::write(
        directory.path().join("header.cpp"),
        r#"#include "typed_api_bindings.h"
#include <type_traits>

static int32_t handle(void *, const empty_t *, wl_delivery_t) { return 0; }
static_assert(std::is_same_v<typed_api_empty_handler_fn, decltype(&handle)>);

int main() {
  empty_t scratch{};
  typed_api_router_t router{};
  router.empty = typed_api_empty_route_t{&scratch, handle, nullptr};
  typed_api_send_result_t result{};
  return result.domain == 0 && router.empty.scratch == &scratch ? 0 : 1;
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
    assert!(
        cxx_status.success(),
        "generated bindings header must be C++20-clean"
    );
}

#[test]
fn generated_names_avoid_c11_macros_and_cxx20_keywords() {
    let directory = tempdir().unwrap();
    let schema = parse_schema(
        r#"version 1;
message Class = 1 {
  optional uint32 template = 1;
}
message Bool = 2 {
  optional uint32 concept = 1;
}
"#,
    )
    .unwrap();
    let model = analyze_schema(&schema).unwrap();
    let generated = generate_c(&model, "keyword_api").unwrap();
    assert!(generated.header.contains("struct class_"));
    assert!(generated.header.contains("uint32_t template_;"));
    assert!(generated.header.contains("struct bool_"));
    fs::write(directory.path().join("keyword_api.h"), generated.header).unwrap();
    fs::write(directory.path().join("keyword_api.c"), generated.source).unwrap();
    fs::write(
        directory.path().join("keyword_api_bindings.h"),
        generated.bindings_header,
    )
    .unwrap();
    fs::write(
        directory.path().join("keyword_api_bindings.c"),
        generated.bindings_source,
    )
    .unwrap();
    fs::write(
        directory.path().join("header.cpp"),
        "#include \"keyword_api_bindings.h\"\nint main() { return 0; }\n",
    )
    .unwrap();

    let c_status = Command::new("cc")
        .args([
            "-std=c11",
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
        .arg(directory.path().join("keyword_api.c"))
        .arg(directory.path().join("keyword_api_bindings.c"))
        .status()
        .unwrap();
    assert!(
        c_status.success(),
        "keyword schema must be strict C11-clean"
    );

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
    assert!(
        cxx_status.success(),
        "keyword schema must be strict C++20-clean"
    );
}
