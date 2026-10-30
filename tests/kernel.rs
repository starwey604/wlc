// SPDX-License-Identifier: Apache-2.0
use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_kernel_profile, analyze_schema, generate_kernel_c, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = "version 1; message Request @id(2) { required int32 input @id(1); required string<31> name @id(2); } message Response @id(3) { required bytes<32> data @id(1); } message Tick @id(4) {}";
const PROFILE: &str = "profile version 2; endpoint { envelope = native_packet; } rpc Execute { request = Request; response = Response; } send Tick { delivery = unreliable; } direct Tick { delivery = unreliable; }";

#[test]
fn removed_contracts_are_errors_and_v1_defaults_are_unchanged() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    for (profile, diagnostic) in [
        (PROFILE.replace("version 2", "version 1"), "version 2"),
        (
            PROFILE.replace(
                "response = Response;",
                "response = Response; guarantee = replay;",
            ),
            "guarantee/replay",
        ),
        (
            PROFILE.replace(
                "response = Response;",
                "response = Response; guarantee = correlate;",
            ),
            "guarantee/replay",
        ),
        (
            PROFILE.replace(
                "response = Response;",
                "response = Response; request_operation_id = input;",
            ),
            "mapped",
        ),
        (
            PROFILE.replace(
                "response = Response;",
                "response = Response; request_delivery = reliable;",
            ),
            "reliable",
        ),
        (
            PROFILE.replace("delivery = unreliable", "delivery = reliable"),
            "reliable",
        ),
        (PROFILE.replace("send Tick", "fifo Tick"), "LATEST/FIFO"),
        (
            PROFILE.replace("native_packet", "cobs_stream"),
            "K1 supports",
        ),
    ] {
        let parsed = parse_binding_profile(&profile).unwrap();
        let error = analyze_kernel_profile(&parsed, &schema)
            .unwrap_err()
            .to_string();
        assert!(error.contains(diagnostic), "{error}");
    }
    assert!(
        parse_binding_profile(&PROFILE.replace(
            "response = Response;",
            "response = Response; deferred = true;"
        ))
        .is_err()
    );
    let legacy = parse_binding_profile(&PROFILE.replace("version 2", "version 1")).unwrap();
    let old = wlc::analyze_binding_profile(&legacy, &schema).unwrap();
    assert_eq!(
        old.rpc_services[0].request_delivery,
        wlc::profile_semantic::DeliveryPolicy::Reliable
    );
    assert!(
        wlc::analyze_binding_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).is_err()
    );
}

#[test]
fn deterministic_composition_and_no_legacy_execution_artifacts() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let rpc = analyze_kernel_profile(
        &parse_binding_profile(
            "profile version 2; rpc Execute { request = Request; response = Response; }",
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let direct = analyze_kernel_profile(&parse_binding_profile("profile version 2; endpoint { envelope = native_packet; } send Tick { delivery = unreliable; } direct Tick { delivery = unreliable; }").unwrap(), &schema).unwrap();
    let a = wlc::compose_kernel_profiles(&[rpc.clone(), direct.clone()]).unwrap();
    let b = wlc::compose_kernel_profiles(&[direct, rpc]).unwrap();
    let a = generate_kernel_c(&schema, &a, "demo").unwrap();
    assert_eq!(a, generate_kernel_c(&schema, &b, "demo").unwrap());
    assert!(a.files["demo_kernel.h"].contains("KERNEL_PROFILE_VERSION 2U"));
    for forbidden in [
        "wl_ctx_t",
        "wl_rpc_server_",
        "wl_rpc_client_",
        "wl_pump_",
        "fingerprint",
        "_runtime.h",
        "reply_token",
    ] {
        assert!(
            !a.files["demo_kernel.c"].contains(forbidden)
                && !a.files["demo_kernel_codecs.h"].contains(forbidden),
            "{forbidden}"
        );
    }
    assert_eq!(a.files.len(), 8);
}

#[test]
fn kernel_cli_generates_versioned_artifacts_and_rejects_reliable() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("demo.wl");
    let profile = directory.path().join("demo.bind.wl");
    let output = directory.path().join("out");
    fs::write(&schema, SCHEMA).unwrap();
    fs::write(&profile, PROFILE).unwrap();
    let run = || {
        assert_cmd::Command::cargo_bin("wlc")
            .unwrap()
            .arg("compile-kernel")
            .arg(&schema)
            .arg("--profile")
            .arg(&profile)
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
    let original = fs::read(output.join("demo_kernel.c")).unwrap();
    assert!(output.join("demo_kernel_deployment.json").is_file());
    assert!(!output.join("demo_runtime.c").exists());
    fs::write(
        &profile,
        PROFILE.replace("delivery = unreliable", "delivery = reliable"),
    )
    .unwrap();
    let failed = run();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("reliable"));
    assert_eq!(original, fs::read(output.join("demo_kernel.c")).unwrap());
}

#[test]
fn different_schema_compiles_warning_clean_and_runs_without_old_core() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_kernel_profile(&parse_binding_profile(PROFILE).unwrap(), &schema).unwrap();
    let generated = generate_kernel_c(&schema, &profile, "demo").unwrap();
    let directory = tempdir().unwrap();
    for (name, content) in generated.files {
        fs::write(directory.path().join(name), content).unwrap();
    }
    fs::write(directory.path().join("test.c"), r#"
#include "demo_kernel.h"
#include <stdlib.h>
static unsigned callbacks;
static wl_packet_submit_t sink(void *u, const uint8_t *p, size_t n, wl_packet_token_t t) {
  (void)u; (void)p; (void)n; (void)t; return WL_PACKET_SENT;
}
static void done(void *u, wl_kernel_result_t r, const response_t *v) {
  (void)u; if (r.outcome != WL_KERNEL_TIMED_OUT || v != NULL) abort(); ++callbacks;
}
int main(void) {
  static demo_kernel_t endpoint; static wl_packet_port_t port; static uint8_t rx[2][2048];
  wl_packet_config_t c = {.instance=99, .rx_storage=&rx[0][0], .rx_unit_size=2048,
    .rx_storage_size=sizeof(rx), .submit=sink};
  demo_kernel_handlers_t handlers = {0};
  if (wl_packet_init(&port,&c) || demo_kernel_init(&endpoint,&port,&handlers,NULL,NULL)) abort();
  request_value_t req = {.has_input=true,.input=INT32_MIN,.has_name=true,.name={.length=3,.data={'a',0,'b'}}};
  wl_kernel_call_handle_t handle;
  if (demo_kernel_execute_start(&endpoint,&req,0,1,done,NULL,&handle)) abort();
  tick_value_t tick = {0};
  if (demo_kernel_send_tick(&endpoint,&tick)) abort();
  if (demo_kernel_step(&endpoint,0) || demo_kernel_step(&endpoint,1) || callbacks != 1) abort();
  if (wl_kernel_close(&endpoint.core) || demo_kernel_step(&endpoint,1) || !wl_kernel_quiescent(&endpoint.core)) abort();
  return 0;
}
"#).unwrap();
    let prototype = root.join("experimental/static_kernel");
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
        .arg(prototype.join("include"))
        .arg("-I")
        .arg(directory.path());
    for file in ["demo.c", "demo_kernel.c", "test.c"] {
        cc.arg(directory.path().join(file));
    }
    for file in ["kernel.c", "packet_port.c", "native_none.c", "rpc_wire.c"] {
        cc.arg(prototype.join("src").join(file));
    }
    let binary = directory.path().join("test");
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
