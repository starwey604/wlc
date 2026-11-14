use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn codegen_contract_is_queryable_without_a_schema() {
    Command::cargo_bin("wlc")
        .unwrap()
        .arg("codegen-contract")
        .assert()
        .success()
        .stdout(format!(
            "{}.{}\n",
            wlc::CODEGEN_CONTRACT_MAJOR,
            wlc::CODEGEN_CONTRACT_MINOR
        ));
}

#[test]
fn version_reports_the_package_release() {
    let output = Command::cargo_bin("wlc")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("wlc {}\n", env!("CARGO_PKG_VERSION"))
    );
}

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
    let manifest =
        fs::read_to_string(output.join("motor_api_manifest.json")).expect("generated manifest");
    assert!(header.contains("#include \"motor_api_values.h\""));
    let values = fs::read_to_string(output.join("motor_api_values.h")).unwrap();
    assert!(values.contains("STATUS_MESSAGE_ID 1U"));
    assert!(values.contains("status_value_t"));
    assert!(source.contains("#include \"motor_api.h\""));
    assert!(!source.contains("wl_send_reliable"));
    assert!(manifest.contains("\"format\": \"wirelink-codegen-manifest-v1\""));
    assert!(manifest.contains("\"module\": \"motor_api\""));
    assert!(manifest.contains("\"binding_profile\": null"));
    assert!(manifest.contains("\"path\": \"motor_api.h\""));
}

#[test]
fn compile_records_bounds_and_validate_rejects_bound_changes() {
    let directory = tempdir().unwrap();
    let previous = directory.path().join("previous.wl");
    let current = directory.path().join("metadata.wl");
    let incompatible = directory.path().join("incompatible.wl");
    let output = directory.path().join("generated");
    fs::write(
        &previous,
        "version 1; message Metadata = 7 { optional string<31> name = 1; optional bytes<8> tag = 2; }",
    )
    .unwrap();
    fs::write(
        &current,
        "version 2; message Metadata = 7 { optional string<31> name = 1; optional bytes<8> tag = 2; optional uint32 revision = 3; }",
    )
    .unwrap();
    fs::write(
        &incompatible,
        "version 2; message Metadata = 7 { optional string<32> name = 1; optional bytes<8> tag = 2; }",
    )
    .unwrap();

    Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "compile",
            current.to_str().unwrap(),
            "--previous",
            previous.to_str().unwrap(),
            "--out-dir",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest = fs::read_to_string(output.join("metadata_manifest.json")).unwrap();
    let header = fs::read_to_string(output.join("metadata_values.h")).unwrap();
    assert!(manifest.contains("\"field\": \"name\""));
    assert!(manifest.contains("\"kind\": \"string\", \"max_length\": 31"));
    assert!(manifest.contains("\"kind\": \"bytes\", \"max_length\": 8"));
    assert!(header.contains("METADATA_HAS_MAX_ENCODED_SIZE 1"));

    let assertion = Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "validate",
            incompatible.to_str().unwrap(),
            "--previous",
            previous.to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&assertion.get_output().stderr).contains("changed wire identity")
    );
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
        "version 2; message Control = 1 { optional float32 timestamp = 1; packed float32 joints[30] = 2; optional float64 clock = 3; optional uint16 channel = 4; }",
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
    assert!(header.contains("uint16_t channel;"));
    assert!(!output.join("control_bindings.h").exists());
    assert!(!output.join("control_bindings.c").exists());
}

#[test]
fn compile_manifest_is_reproducible_across_output_directories() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("stable.wl");
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    fs::write(
        &schema,
        "version 1; message State = 1 { optional uint32 sequence = 1; }",
    )
    .unwrap();

    for output in [&first, &second] {
        Command::cargo_bin("wlc")
            .unwrap()
            .args([
                "compile",
                schema.to_str().unwrap(),
                "--out-dir",
                output.to_str().unwrap(),
            ])
            .assert()
            .success();
    }

    assert_eq!(
        fs::read(first.join("stable_manifest.json")).unwrap(),
        fs::read(second.join("stable_manifest.json")).unwrap()
    );
}

#[test]
fn release_smoke_fixture_uses_the_current_kernel_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = tempdir().unwrap();
    Command::cargo_bin("wlc")
        .unwrap()
        .arg("compile-kernel")
        .arg(root.join("tests/fixtures/release_smoke.wl"))
        .arg("--profile")
        .arg(root.join("tests/fixtures/release_smoke.bind.wl"))
        .arg("--out-dir")
        .arg(directory.path())
        .assert()
        .success();
    let manifest: String =
        fs::read_to_string(directory.path().join("release_smoke_kernel_manifest.json")).unwrap();
    assert!(manifest.contains("\"minor\": 13"));
    for name in [
        "release_smoke_kernel.h",
        "release_smoke_kernel.c",
        "release_smoke_kernel_codecs.h",
        "release_smoke_values.h",
    ] {
        assert!(directory.path().join(name).is_file(), "missing {name}");
    }
}
