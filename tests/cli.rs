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
    assert!(header.contains("STATUS_MESSAGE_ID 1U"));
    assert!(source.contains("#include \"motor_api.h\""));
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
