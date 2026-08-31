use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn compile_writes_named_c_artifacts() {
    let directory = tempdir().expect("temporary directory");
    let schema = directory.path().join("motor_api.wl");
    let output = directory.path().join("generated");
    fs::write(
        &schema,
        "version 1; message Status = 1 { optional bool ready = 1; }",
    )
    .expect("write schema");

    Command::cargo_bin("wlc")
        .expect("wlc binary")
        .args([
            "compile",
            schema.to_str().unwrap(),
            "--out-dir",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let header = fs::read_to_string(output.join("motor_api.h")).expect("generated header");
    let source = fs::read_to_string(output.join("motor_api.c")).expect("generated source");
    let bindings_header =
        fs::read_to_string(output.join("motor_api_bindings.h")).expect("generated bindings header");
    let bindings_source =
        fs::read_to_string(output.join("motor_api_bindings.c")).expect("generated bindings source");
    assert!(header.contains("STATUS_MESSAGE_ID 1U"));
    assert!(source.contains("#include \"motor_api.h\""));
    assert!(bindings_header.contains("motor_api_dispatch_event"));
    assert!(bindings_header.contains("motor_api_status_send_reliable"));
    assert!(bindings_source.contains("#include \"motor_api_bindings.h\""));
    assert!(!source.contains("wl_send_reliable"));
}

#[test]
fn validate_accepts_a_compatible_predecessor() {
    let directory = tempdir().expect("temporary directory");
    let previous = directory.path().join("previous.wl");
    let current = directory.path().join("current.wl");
    fs::write(
        &previous,
        "version 1; message Status = 1 { optional uint32 sequence = 1; }",
    )
    .expect("write predecessor");
    fs::write(
        &current,
        "version 2; message Status = 1 { optional uint32 sequence = 1; optional bool ready = 2; }",
    )
    .expect("write current schema");

    Command::cargo_bin("wlc")
        .expect("wlc binary")
        .args([
            "validate",
            current.to_str().unwrap(),
            "--previous",
            previous.to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn compile_and_validate_support_dense_numeric_fields() {
    let directory = tempdir().expect("temporary directory");
    let previous = directory.path().join("previous.wl");
    let current = directory.path().join("control.wl");
    let output = directory.path().join("generated");
    fs::write(
        &previous,
        "version 1; message Control = 1 { optional float32 timestamp = 1; packed float32 joints[30] = 2; }",
    )
    .expect("write predecessor");
    fs::write(
        &current,
        "version 2; message Control = 1 { optional float32 timestamp = 1; packed float32 joints[30] = 2; optional float64 clock = 3; }",
    )
    .expect("write current schema");

    Command::cargo_bin("wlc")
        .expect("wlc binary")
        .args([
            "compile",
            current.to_str().unwrap(),
            "--out-dir",
            output.to_str().unwrap(),
            "--previous",
            previous.to_str().unwrap(),
        ])
        .assert()
        .success();

    let header = fs::read_to_string(output.join("control.h")).expect("generated header");
    assert!(header.contains("float joints[30];"));
    assert!(header.contains("double clock;"));
    assert!(output.join("control_bindings.h").is_file());
    assert!(output.join("control_bindings.c").is_file());
}

#[test]
fn optional_profile_validation_does_not_change_generated_artifacts() {
    let directory = tempdir().expect("temporary directory");
    let schema = directory.path().join("control.wl");
    let profile = directory.path().join("device.bind.wl");
    let plain_output = directory.path().join("plain");
    let profiled_output = directory.path().join("profiled");
    fs::write(
        &schema,
        r#"version 1;
enum Status = 1 { OK = 0; FAILED = 1; }
message Control = 2 { packed float32 values[6] = 1; }
message Request = 3 { optional uint32 operation_id = 1; }
message Response = 4 { optional uint32 operation_id = 1; optional Status status = 2; }
"#,
    )
    .expect("write schema");
    fs::write(
        &profile,
        r#"profile version 1;
latest Control { delivery = unreliable; }
rpc Start {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#,
    )
    .expect("write profile");

    Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "compile",
            schema.to_str().unwrap(),
            "--out-dir",
            plain_output.to_str().unwrap(),
        ])
        .assert()
        .success();
    Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "compile",
            schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--out-dir",
            profiled_output.to_str().unwrap(),
        ])
        .assert()
        .success();

    for artifact in [
        "control.h",
        "control.c",
        "control_bindings.h",
        "control_bindings.c",
    ] {
        assert_eq!(
            fs::read(plain_output.join(artifact)).unwrap(),
            fs::read(profiled_output.join(artifact)).unwrap(),
            "profile must not affect {artifact}"
        );
    }
    assert_eq!(fs::read_dir(&profiled_output).unwrap().count(), 4);
}

#[test]
fn profile_diagnostics_point_at_the_sidecar_source() {
    let directory = tempdir().expect("temporary directory");
    let schema = directory.path().join("control.wl");
    let profile = directory.path().join("invalid.bind.wl");
    fs::write(
        &schema,
        "version 1; message Control = 1 { optional bytes payload = 1; }",
    )
    .unwrap();
    fs::write(
        &profile,
        "profile version 1;\nlatest Control { delivery = unreliable; }\n",
    )
    .unwrap();

    let assertion = Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "validate",
            schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr);
    assert!(stderr.contains("invalid.bind.wl"));
    assert!(stderr.contains("borrowed or caller-owned storage"));
}
