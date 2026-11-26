// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_node_c, parse_schema};

const SCHEMA: &str = "version 1; message Request @id(2) { required int32 input @id(1); required string<31> name @id(2); } message Response @id(3) { required bytes<32> data @id(1); } message Tick @id(4) {}";
const NODE: &str = "rpc Execute(Request) -> Response; node demo { call Execute; serve Execute; send Tick; recv Tick; }";

fn generate(source: &str) -> wlc::GeneratedNodeC {
    let ast = parse_schema(source).unwrap();
    let schema = analyze_schema(&ast).unwrap();
    let nodes = wlc::node::analyze_nodes(&ast, &schema).unwrap();
    generate_node_c(&schema, &nodes, "demo").unwrap()
}

fn compile_and_run(generated: wlc::GeneratedNodeC, program: &str, connector: Option<&str>) {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let directory = tempdir().unwrap();
    for (name, content) in generated.files {
        fs::write(directory.path().join(name), content).unwrap();
    }
    fs::write(directory.path().join("test.c"), program).unwrap();
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args([
        "-std=c11",
        "-Os",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]);
    }
    cc.arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path());
    for file in ["demo.c", "test.c"] {
        cc.arg(directory.path().join(file));
    }
    for file in ["codec.c", "node.c", "endpoint.c", "packet_port.c"] {
        cc.arg(root.join("src").join(file));
    }
    if let Some(connector) = connector {
        cc.arg(root.join("src/packet_stream.c"));
        cc.arg(root.join("src").join(format!("{connector}_stream.c")));
        if connector == "cobs_crc16" {
            cc.arg(root.join("src/cobs.c")).arg(root.join("src/crc.c"));
        }
    }
    let binary = directory.path().join("test");
    // Deliberately no function/data sections or linker garbage collection:
    // selected-node programs must not depend on unused handler elimination.
    let output = cc.arg("-o").arg(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn removed_contracts_are_errors_on_the_single_frontend() {
    for removed in [
        "profile version 1;",
        "profile version 2;",
        "endpoint { envelope=native_packet; }",
        "node demo { guarantee = replay; }",
        "node demo { guarantee = correlate; }",
        "node demo { request_operation_id = input; }",
        "node demo { request_delivery = reliable; }",
        "node demo { send Tick { delivery = reliable; } }",
        "node demo { fifo Tick; }",
        "node demo { latest Tick; }",
        "node demo { direct Tick; }",
        "node demo { rpc_role = server; }",
        "node demo { deferred = true; }",
        "node demo { io = embedded; }",
        "node demo { io = external; }",
    ] {
        let source = format!("{SCHEMA} {removed}");
        let rejected = match parse_schema(&source) {
            Err(_) => true,
            Ok(ast) => wlc::node::analyze_nodes(&ast, &analyze_schema(&ast).unwrap()).is_err(),
        };
        assert!(rejected, "accepted removed syntax: {removed}");
    }
    generate(&format!("{SCHEMA} {NODE}"));
}

#[test]
fn deterministic_composition_and_no_legacy_execution_artifacts() {
    let a = generate(&format!("{SCHEMA} {NODE}"));
    let b = generate(&format!(
        "{SCHEMA} node demo {{ recv Tick; serve Execute; send Tick; call Execute; }} rpc Execute(Request) -> Response;"
    ));
    assert_eq!(a, b);
    for forbidden in [
        "wl_ctx_t",
        "wl_rpc_server_",
        "wl_rpc_client_",
        "wl_pump_",
        "fingerprint",
        "_runtime.h",
        "reply_token",
        "_kernel.h",
    ] {
        assert!(
            !a.files["demo.c"].contains(forbidden) && !a.files["demo.h"].contains(forbidden),
            "{forbidden}"
        );
    }
    assert!(a.files["demo.h"].contains("extern \"C\" {"));
    assert_eq!(a.files.len(), 3);
    assert!(a.files["demo_manifest.json"].contains("\"nodes\""));
}

#[test]
fn node_cli_generates_versioned_artifacts_and_rejects_reliable() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("demo.wl");
    let output = directory.path().join("out");
    fs::write(&schema, format!("{SCHEMA} {NODE}")).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg("compile")
            .arg(&schema)
            .arg("--out-dir")
            .arg(&output)
            .output()
            .unwrap()
    };
    let success = run();
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let original = fs::read(output.join("demo.c")).unwrap();
    assert!(output.join("demo_manifest.json").is_file());
    assert!(!output.join("demo_runtime.c").exists());
    fs::write(
        &schema,
        format!(
            "{SCHEMA} {}",
            NODE.replace("send Tick;", "send Tick { delivery = reliable; }")
        ),
    )
    .unwrap();
    assert!(!run().status.success());
    assert_eq!(original, fs::read(output.join("demo.c")).unwrap());
}

#[test]
fn dynamic_external_node_compiles_and_times_out_without_old_core() {
    let generated = generate(&format!(
        "{SCHEMA} {}",
        NODE.replace("node demo {", "node demo { handlers = dynamic;")
    ));
    compile_and_run(
        generated,
        r#"
#include "demo.h"
#include <assert.h>
static unsigned callbacks;
static wl_packet_submit_t sink(wl_transport_t *u, const uint8_t *p, size_t n, wl_packet_token_t t) {
  (void)u; (void)p; (void)n; (void)t; return WL_PACKET_SENT;
}
static void done(void *u, wl_rpc_result_t r, const response_t *v) {
  assert(u == &callbacks && r.outcome == WL_RPC_TIMED_OUT && v == NULL); ++callbacks;
}
static uint8_t rx[2][128];
static int invalid_storage = 1;
static wl_err_t configure_rx(wl_transport_t *self, size_t minimum, wl_transport_rx_override_t *slots) {
  (void)self;
  assert(minimum == DEMO_RX_WIRE_MAX + 1U);
  *slots = (wl_transport_rx_override_t){invalid_storage ? NULL : rx[0], sizeof(rx), sizeof(rx[0])};
  return WL_OK;
}
static wl_err_t attach(wl_transport_t *self, const wl_transport_attach_t *info) {
  (void)self;
  assert(wl_packet_rx_unit_size(info->port) == sizeof(rx[0]));
  assert(info->port->config.rx_storage == rx[0]);
  return WL_OK;
}
static wl_err_t stop(wl_transport_t *self) { (void)self; return WL_OK; }
int main(void) {
  static demo_node_t endpoint;
  static const wl_transport_ops_t ops = {.configure_rx=configure_rx,.attach=attach,.submit=sink,.stop=stop};
  wl_transport_t transport = {.ops=&ops};
  demo_handlers_t handlers = {0};
  assert(demo_bind_instance(&endpoint,99,&transport,&handlers,NULL) == WL_ERR_INVALID_ARG);
  invalid_storage = 0;
  assert(demo_bind_instance(&endpoint,99,&transport,&handlers,NULL) == WL_OK);
  request_t req = {.input=INT32_MIN,.name={.length=3,.data={'a',0,'b'}}};
  assert(demo_call_execute_ex(&endpoint,&req,100000U,1U,done,&callbacks,NULL) == WL_OK);
  tick_t tick = {0};
  assert(demo_send_tick(&endpoint,&tick) == WL_OK);
  assert(wl_node_step(&endpoint.node,100000U) == WL_OK && callbacks == 0U);
  assert(wl_node_step(&endpoint.node,100001U) == WL_OK && callbacks == 1U);
  assert(wl_node_close(&endpoint.node, 100001U) == WL_OK && wl_node_step(&endpoint.node,100001U) == WL_OK);
  assert(wl_node_quiescent(&endpoint.node));
  return 0;
}
"#,
        None,
    );
}

#[test]
fn bounded_bytes_decode_into_private_callback_values_only() {
    let generated = generate(
        "message Inner @id(1) { bytes<64> data @id(1); }
         message Packet @id(2) { Inner inner @id(1); uint32 seq @id(2); }
         node sub { recv Packet; }",
    );
    assert!(generated.files["demo.c"].contains("WLC_NODE_DECODE(packet, frame->payload"));
    compile_and_run(
        generated,
        r#"
#include "demo.h"
#include <assert.h>
#include <string.h>
static unsigned calls;
void sub_on_packet(sub_node_t *self, const packet_t *p) {
  (void)self;
  assert(p->seq == 7 && p->inner.data.length == 3);
  assert(memcmp(p->inner.data.data, "a\0b", 3) == 0);
  ++calls;
}
int main(void) {
  sub_node_t sub;
  assert(sub_bind_sync(&sub) == WL_OK);
  /* Out of order, with an unknown field; the nested bytes are still owned. */
  uint8_t frame[] = {WL_WIRE_MAGIC, WL_WIRE_MSG, 2, 0, 16,7, 24,1, 10,5, 10,3,'a',0,'b', 16,1};
  /* A duplicate after the bytes have been decoded must not publish a partial
   * value or poison the next frame. The public decoder leaves out intact. */
  packet_t out;
  unsigned char before[sizeof(out)];
  memset(&out, 0xa5, sizeof(out)); memcpy(before, &out, sizeof(out));
  assert(packet_decode(frame+4, sizeof(frame)-4, &out) == WL_CODEC_ERR_DUPLICATE_FIELD);
  assert(memcmp(&out, before, sizeof(out)) == 0);
  assert(wl_node_rx_packet(&sub.node, frame, sizeof(frame)) == WL_OK);
  assert(wl_node_step(&sub.node, 0) == WL_ERR_CORRUPT_PAYLOAD && calls == 0);
  assert(wl_node_rx_packet(&sub.node, frame, sizeof(frame)-2) == WL_OK);
  assert(wl_node_step(&sub.node, 1) == WL_OK && calls == 1);
  return 0;
}
"#,
        None,
    );
}

#[test]
fn selected_stream_connectors_run_static_rpc_and_abort_half_frames() {
    for (framing, connector) in [("cobs_crc16", "cobs_crc16"), ("length16", "length16_none")] {
        let generated = generate(&format!(
            "{SCHEMA} {}",
            NODE.replace("node demo {", &format!("node demo {{ framing = {framing};"))
        ));
        assert!(
            generated.files["demo_manifest.json"].contains(&format!("\"framing\":\"{framing}\""))
        );
        assert!(
            generated.files["demo.c"].contains(&format!("wl_packet_stream_{connector}_submit"))
        );
        assert!(!generated.files["demo.c"].contains("wl_frame_encode"));
        assert!(
            !generated.files["demo.c"].contains(if connector == "cobs_crc16" {
                "length16_none"
            } else {
                "cobs_crc16"
            })
        );
        compile_and_run(
            generated,
            r#"
#include "demo.h"
#include <assert.h>
#include <string.h>
static unsigned completed, effects;
static uint8_t saved[256]; static size_t saved_length;
wl_err_t demo_tx_write(demo_node_t *self, const uint8_t *data, size_t length) {
  size_t consumed = 0;
  assert(length <= sizeof(saved)); memcpy(saved, data, length); saved_length=length;
  wl_err_t error = wl_node_rx_feed(&self->node,data,length,&consumed);
  assert(error == WL_OK && consumed == length);
  return WL_OK;
}
void demo_on_tick(demo_node_t *self, const tick_t *tick) { (void)self; (void)tick; }
int32_t demo_on_execute(demo_node_t *self, const request_t *request, response_t *response) {
  (void)self;
  assert(request->input == INT32_MIN && request->name.length == 3 && !memcmp(request->name.data,"a\0b",3));
  ++effects; response->data.length = 32;
  for (unsigned i = 0; i < 32; ++i) response->data.data[i] = (uint8_t)i;
  return 0;
}
static void done(demo_node_t *self, wl_rpc_result_t result, const response_t *response) {
  (void)self;
  assert(result.outcome == WL_RPC_SUCCESS && response && response->data.length == 32);
  for (unsigned i=0; i<32; ++i) assert(response->data.data[i] == (uint8_t)i);
  ++completed;
}
int main(void) {
  static demo_node_t endpoint;
  assert(demo_bind_sync(&endpoint, demo_tx_write) == WL_OK);
  request_t request = {.input=INT32_MIN,.name={.length=3,.data={'a',0,'b'}}};
  assert(demo_call_execute(&endpoint,&request,done) == WL_OK);
  memset(&request,0,sizeof(request));
  for (unsigned i=0; i<10 && !completed; ++i) assert(wl_node_step(&endpoint.node,100000U) == WL_OK);
  assert(completed == 1 && effects == 1);
  size_t consumed=0;
  assert(wl_node_rx_feed(&endpoint.node,saved,saved_length/2,&consumed) == WL_OK && consumed==saved_length/2);
  assert(wl_node_close(&endpoint.node, 100001U) == WL_OK && wl_node_step(&endpoint.node,100001U) == WL_OK);
  assert(wl_node_quiescent(&endpoint.node));
  return 0;
}
"#,
            Some(connector),
        );
    }
}

#[test]
fn pure_receive_stream_and_tx_ready_backpressure_recovery() {
    for (framing, connector) in [
        ("native", None),
        ("cobs_crc16", Some("cobs_crc16")),
        ("length16", Some("length16_none")),
    ] {
        let generated = generate(&format!(
            "message Tick @id(4) {{ uint32 value @id(1); uint32 self @id(2); }} node pub {{ framing={framing}; send Tick; }} node sub {{ framing={framing}; recv Tick; }}"
        ));
        assert!(!generated.files["demo.h"].contains("sub_tx_write"));
        assert!(generated.files["demo.h"].contains("PUB_REQUEST_FRAME_MAX 0U"));
        assert!(generated.files["demo.h"].contains("SUB_REPLY_FRAME_MAX 0U"));
        let receive = if connector.is_some() {
            "size_t consumed=0; wl_err_t error=wl_node_rx_feed(&subscriber.node,data,length,&consumed); assert(consumed==length); return error;"
        } else {
            "return wl_node_rx_packet(&subscriber.node,data,length);"
        };
        compile_and_run(
            generated,
            &format!(
                r#"
#include "demo.h"
#include <assert.h>
static sub_node_t subscriber;
static unsigned attempts, deliveries, notifications;
static wl_transport_t transport;
static wl_err_t attach(wl_transport_t *self, const wl_transport_attach_t *info) {{
  assert(self == &transport && info->port == &subscriber.node.port);
  return WL_OK;
}}
static wl_err_t stop(wl_transport_t *self) {{ assert(self == &transport); return WL_OK; }}
static void notify(wl_transport_t *self) {{
  assert(self == &transport);
  ++notifications;
}}
wl_err_t pub_tx_write(pub_node_t *self, const uint8_t *data, size_t length) {{
  (void)self; if (++attempts==1U) return WL_ERR_WOULD_BLOCK;
  {receive}
}}
void sub_on_tick(sub_node_t *self, const tick_t *tick) {{
  assert(self==&subscriber && tick->value==42U && tick->self==7U); ++deliveries;
}}
int main(void) {{
  pub_node_t publisher;
  assert(pub_bind_sync(&publisher, NULL)==WL_ERR_INVALID_ARG);
  assert(pub_bind_sync(&publisher, pub_tx_write)==WL_OK);
  static const wl_transport_ops_t ops = {{.attach=attach,.notify=notify,.stop=stop}};
  transport.ops = &ops;
  assert(sub_bind(&subscriber,&transport)==WL_OK);
  wl_node_tx_ready(&subscriber.node);
  assert(notifications == 1U);
  assert(pub_send_tick_val(&publisher,42U,7U)==WL_OK);
  assert(wl_node_step(&publisher.node,0U)==WL_OK && attempts==1U);
  assert(wl_node_step(&publisher.node,1U)==WL_OK && attempts==1U);
  wl_node_tx_ready(&publisher.node);
  assert(wl_node_step(&publisher.node,2U)==WL_OK && attempts==2U);
  assert(wl_node_step(&subscriber.node,2U)==WL_OK && deliveries==1U);
  assert(wl_node_close(&publisher.node, 100001U)==WL_OK && wl_node_close(&subscriber.node, 100001U)==WL_OK);
  assert(wl_node_step(&publisher.node,3U)==WL_OK && wl_node_step(&subscriber.node,3U)==WL_OK);
  assert(wl_node_quiescent(&publisher.node) && wl_node_quiescent(&subscriber.node));
  return 0;
}}
"#
            ),
            connector,
        );
    }
}

#[test]
fn selected_client_links_without_server_handlers_under_default_link_options() {
    let ast = parse_schema(&format!("{SCHEMA} rpc Execute(Request)->Response; node server {{ serve Execute; }} node client {{ call Execute; }}")).unwrap();
    let schema = analyze_schema(&ast).unwrap();
    let nodes = wlc::node::analyze_nodes(&ast, &schema).unwrap();
    let selected = wlc::node::select_nodes(&nodes, Some("client"), false).unwrap();
    let generated = generate_node_c(&schema, &selected, "demo").unwrap();
    assert!(!generated.files["demo.h"].contains("server_on_execute"));
    assert!(!generated.files["demo.h"].contains("request_decode("));
    assert!(!generated.files["demo.h"].contains("response_encode("));
    compile_and_run(
        generated,
        r#"
#include "demo.h"
#include <assert.h>
wl_err_t client_tx_write(client_node_t *self,const uint8_t *data,size_t length) {
  (void)self; (void)data; (void)length; return WL_OK;
}
int main(void) { client_node_t client; assert(client_bind_sync(&client, client_tx_write)==WL_OK);
  assert(wl_node_close(&client.node, 100001U)==WL_OK && wl_node_step(&client.node,0U)==WL_OK && wl_node_quiescent(&client.node)); return 0; }
"#,
        None,
    );
}

#[test]
fn default_frontend_rejects_legacy_outputs_and_reports_current_plan() {
    let dir = tempdir().unwrap();
    let schema = dir.path().join("demo.wl");
    let output = dir.path().join("out");
    fs::create_dir(&output).unwrap();
    fs::write(&schema, format!("{SCHEMA} {NODE}")).unwrap();
    fs::write(output.join("demo_runtime.c"), "preserve user bytes").unwrap();
    let run = |command: &str| {
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg(command)
            .arg(&schema)
            .arg("--out-dir")
            .arg(&output)
            .output()
            .unwrap()
    };
    let failed = run("compile");
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("legacy execution artifacts"));
    assert_eq!(
        fs::read_to_string(output.join("demo_runtime.c")).unwrap(),
        "preserve user bytes"
    );
    assert!(!output.join("demo.c").exists());
    for command in ["compile-runtime", "compile-kernel"] {
        assert!(!run(command).status.success());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_wlc"))
        .arg("plan")
        .arg(&schema)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("wirelink-node-plan-v1") && text.contains("\"request_frame_capacity\""));
    assert!(!text.contains("replay_methods") && !text.contains("pending_response_methods"));
}

#[test]
fn cli_and_build_tools_share_c_module_name_normalization() {
    let dir = tempdir().unwrap();
    let schema = dir.path().join("switch.wl");
    fs::write(&schema, "message Empty @id(1) {}").unwrap();
    let name = Command::new(env!("CARGO_BIN_EXE_wlc"))
        .arg("module-name")
        .arg(&schema)
        .output()
        .unwrap();
    assert!(name.status.success());
    assert_eq!(name.stdout, b"switch_\n");
    let out = dir.path().join("out");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg("compile")
            .arg(&schema)
            .arg("--out-dir")
            .arg(&out)
            .status()
            .unwrap()
            .success()
    );
    assert!(out.join("switch_.c").exists() && out.join("switch_.h").exists());
    assert!(
        fs::read_to_string(out.join("switch_.c"))
            .unwrap()
            .contains("#include \"switch_.h\"")
    );
}
