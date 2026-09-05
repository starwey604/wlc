use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

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
fn compile_help_describes_profile_runtime_generation() {
    let output = Command::cargo_bin("wlc")
        .unwrap()
        .args(["compile", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--profile also generates the application runtime"));
    assert!(stdout.contains("deterministic manifest"));
    assert!(stdout.contains("generate <module>_runtime.h/.c"));
    assert!(stdout.contains("Destination directory for generated artifacts"));
}

#[test]
fn compile_runtime_writes_only_named_profile_artifacts() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("control.wl");
    let profile = directory.path().join("device.bind.wl");
    let output = directory.path().join("generated");
    fs::write(
        &schema,
        "version 1; message State = 1 { optional uint32 sequence = 1; }",
    )
    .unwrap();
    fs::write(
        &profile,
        "profile version 1; latest State { delivery = unreliable; }",
    )
    .unwrap();

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
    let codec_manifest = fs::read(output.join("control_manifest.json")).unwrap();

    Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "compile-runtime",
            schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--runtime-name",
            "device_api",
            "--out-dir",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read(output.join("control_manifest.json")).unwrap(),
        codec_manifest
    );
    let header = fs::read_to_string(output.join("device_api_runtime.h")).unwrap();
    let source = fs::read_to_string(output.join("device_api_runtime.c")).unwrap();
    let manifest = fs::read_to_string(output.join("device_api_runtime_manifest.json")).unwrap();
    assert!(header.contains("#include \"control_bindings.h\""));
    assert!(header.contains("device_api_runtime_t"));
    assert!(source.contains("device_api_runtime_dispatch_event"));
    assert!(manifest.contains("\"module\": \"device_api\""));
    assert!(manifest.contains("\"binding_profile\": \"0x"));
    assert!(manifest.contains("\"path\": \"device_api_runtime.c\""));
    assert_eq!(fs::read_dir(output).unwrap().count(), 8);
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
    let bindings_header =
        fs::read_to_string(output.join("motor_api_bindings.h")).expect("generated bindings header");
    let bindings_source =
        fs::read_to_string(output.join("motor_api_bindings.c")).expect("generated bindings source");
    let manifest =
        fs::read_to_string(output.join("motor_api_manifest.json")).expect("generated manifest");
    assert!(header.contains("STATUS_MESSAGE_ID 1U"));
    assert!(source.contains("#include \"motor_api.h\""));
    assert!(bindings_header.contains("motor_api_dispatch_event"));
    assert!(bindings_header.contains("motor_api_status_send("));
    assert!(bindings_source.contains("#include \"motor_api_bindings.h\""));
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
    let header = fs::read_to_string(output.join("metadata.h")).unwrap();
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
    let runtime_header = fs::read_to_string(profiled_output.join("control_runtime.h")).unwrap();
    let runtime_source = fs::read_to_string(profiled_output.join("control_runtime.c")).unwrap();
    assert!(runtime_header.contains("CONTROL_SCHEMA_IDENTITY"));
    assert!(runtime_header.contains("wl_latest_t *control_latest;"));
    assert!(runtime_source.contains("control_runtime_dispatch_event"));
    let plain_manifest = fs::read_to_string(plain_output.join("control_manifest.json")).unwrap();
    let profiled_manifest =
        fs::read_to_string(profiled_output.join("control_manifest.json")).unwrap();
    assert!(plain_manifest.contains("\"binding_profile\": null"));
    assert!(profiled_manifest.contains("\"binding_profile\": \"0x"));
    assert!(profiled_manifest.contains("\"path\": \"control_runtime.c\""));
    assert_ne!(plain_manifest, profiled_manifest);
    assert_eq!(fs::read_dir(&plain_output).unwrap().count(), 5);
    assert_eq!(fs::read_dir(&profiled_output).unwrap().count(), 7);
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

#[test]
fn identity_command_reports_stable_schema_and_profile_values() {
    let directory = tempdir().expect("temporary directory");
    let schema_path = directory.path().join("control.wl");
    let profile_path = directory.path().join("device.bind.wl");
    let schema_source = "version 1; message Control = 1 { packed float32 values[6] = 1; }";
    let profile_source = "profile version 1; latest Control { delivery = unreliable; }";
    fs::write(&schema_path, schema_source).unwrap();
    fs::write(&profile_path, profile_source).unwrap();

    let schema = wlc::analyze_schema(&wlc::parse_schema(schema_source).unwrap()).unwrap();
    let profile = wlc::analyze_binding_profile(
        &wlc::parse_binding_profile(profile_source).unwrap(),
        &schema,
    )
    .unwrap();
    let output = Command::cargo_bin("wlc")
        .unwrap()
        .args([
            "identity",
            schema_path.to_str().unwrap(),
            "--profile",
            profile_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "identity algorithm: {}\nschema identity: 0x{:016x}\nbinding profile identity: 0x{:016x}\n",
            wlc::IDENTITY_ALGORITHM,
            wlc::schema_identity(&schema),
            wlc::binding_profile_identity(&profile)
        )
    );
}
