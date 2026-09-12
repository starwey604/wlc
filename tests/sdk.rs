// SPDX-License-Identifier: Apache-2.0
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::tempdir;
use wlc::{
    SdkOptions, analyze_binding_profile, analyze_schema, generate_sdk, parse_binding_profile,
    parse_schema,
};

const SCHEMA: &str = include_str!("fixtures/sdk/device.wl");
const PROFILE: &str = include_str!("fixtures/sdk/device.bind.wl");
const SIMPLE: &str =
    "version 1; message Request @id(1) {} message Response @id(2) {} message Notice @id(3) {}";
const RPC: &str = "profile version 1; rpc Call { request = Request; response = Response; }";

fn sdk(schema: &str, profile: &str) -> Result<wlc::GeneratedSdk, wlc::SdkCodegenError> {
    let model = analyze_schema(&parse_schema(schema).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(profile).unwrap(), &model).unwrap();
    generate_sdk(
        &model,
        &profile,
        &SdkOptions {
            name: "device".into(),
            package_version: "0.1.0.dev1".into(),
        },
    )
}

fn write_sdk(sdk: &wlc::GeneratedSdk, directory: &Path) {
    for (name, contents) in &sdk.files {
        let path = directory.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

#[test]
fn project_is_deterministic_and_preserves_the_existing_c_artifacts() {
    let generated = sdk(SCHEMA, PROFILE).unwrap();
    assert_eq!(generated, sdk(SCHEMA, PROFILE).unwrap());
    let model = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let c = wlc::generate_c(&model, "device").unwrap();
    assert_eq!(generated.files["generated/device.c"], c.source);
    assert_eq!(
        generated.files["generated/device_values.h"],
        c.values_header
    );
    assert!(generated.files.contains_key("python/device_sdk/py.typed"));
    assert!(generated.files["wlc-sdk-manifest.json"].contains("python/device_sdk/__init__.py"));
    assert!(generated.files["cmake/DeviceSdkConfig.cmake.in"].starts_with("@PACKAGE_INIT@"));
}

#[test]
fn rejects_unsupported_profiles_before_emission() {
    for (profile, diagnostic) in [
        (
            "profile version 1; send Request { delivery = unreliable; }",
            "managed RPC",
        ),
        (
            "profile version 1; endpoint { rpc_role = server; } rpc Call { request = Request; response = Response; }",
            "client-capable",
        ),
        (
            "profile version 1; endpoint { envelope = cobs_stream; } rpc Call { request = Request; response = Response; }",
            "envelope",
        ),
        (
            "profile version 1; send Notice { delivery = unreliable; } rpc Call { request = Request; response = Response; }",
            "RPC-only",
        ),
    ] {
        assert!(
            sdk(SIMPLE, profile)
                .unwrap_err()
                .to_string()
                .contains(diagnostic)
        );
    }
}

#[test]
fn rejects_unbounded_and_oversized_messages_with_a_diagnostic() {
    for (field, diagnostic) in [
        ("optional string label @id(1);", "Request.label"),
        ("repeated uint32 values @id(1);", "Request.values"),
        ("required bytes<2048> data @id(1);", "2048-byte"),
    ] {
        let schema =
            format!("version 1; message Request @id(1) {{ {field} }} message Response @id(2) {{}}");
        assert!(
            sdk(&schema, RPC)
                .unwrap_err()
                .to_string()
                .contains(diagnostic)
        );
    }
}

#[test]
fn rejects_public_names_that_collide_after_mapping() {
    for (schema, profile) in [
        (
            "version 1; message Client @id(3) {} message Request @id(1) {} message Response @id(2) {}",
            RPC,
        ),
        (
            "version 1; message Request @id(1) { optional int32 value @id(1) [default = 1]; optional int32 value_or_default @id(2); } message Response @id(2) {}",
            RPC,
        ),
        (
            "version 1; message Request @id(1) { optional int32 from @id(1); optional int32 from_ @id(2); } message Response @id(2) {}",
            RPC,
        ),
        (
            SIMPLE,
            "profile version 1; rpc Close { request = Request; response = Response; }",
        ),
    ] {
        assert!(
            sdk(schema, profile)
                .unwrap_err()
                .to_string()
                .contains("collision")
        );
    }
}

#[test]
fn rejects_unsafe_project_options() {
    let model = analyze_schema(&parse_schema(SIMPLE).unwrap()).unwrap();
    let profile = analyze_binding_profile(&parse_binding_profile(RPC).unwrap(), &model).unwrap();
    for name in [
        "../escape",
        "bad-name",
        "Upper",
        "_private",
        "class",
        "std",
        "",
        "a__b",
    ] {
        assert!(
            generate_sdk(
                &model,
                &profile,
                &SdkOptions {
                    name: name.into(),
                    package_version: "1.0.0".into()
                }
            )
            .is_err()
        );
    }
    for version in [
        "1",
        "1.0",
        "01.0.0",
        "1.0.0\n",
        "1.0.0.dev",
        "1.0.0;evil",
        "1.0.0.dev1.dev2",
    ] {
        assert!(
            generate_sdk(
                &model,
                &profile,
                &SdkOptions {
                    name: "device".into(),
                    package_version: version.into()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn cpp_values_round_trip_through_the_real_c_codec() {
    let directory = tempdir().unwrap();
    let sdk = sdk(SCHEMA, PROFILE).unwrap();
    write_sdk(&sdk, directory.path());
    fs::write(
        directory.path().join("test.cpp"),
        include_str!("fixtures/sdk/values.cpp"),
    )
    .unwrap();
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned()
        });
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
        "-c",
    ])
    .arg(directory.path().join("generated/device.c"))
    .arg("-I")
    .arg(root.join("include"))
    .arg("-o")
    .arg(directory.path().join("codec.o"));
    let mut cxx = Command::new("c++");
    cxx.args(["-std=c++20", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg(directory.path().join("test.cpp"))
        .arg(directory.path().join("codec.o"));
    for include in [
        root.join("include"),
        directory.path().join("include"),
        directory.path().join("generated"),
        directory.path().join("src"),
    ] {
        cxx.arg("-I").arg(include);
    }
    let executable = directory.path().join("test");
    cxx.arg("-o").arg(&executable);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        for command in [&mut cc, &mut cxx] {
            command.args([
                "-fsanitize=address,undefined",
                "-fno-omit-frame-pointer",
                "-g",
            ]);
        }
    }
    for command in [&mut cc, &mut cxx, &mut Command::new(&executable)] {
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{command:?}\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn cli_is_idempotent_and_requires_explicit_overwrite() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("device.wl");
    let profile = directory.path().join("device.bind.wl");
    let output = directory.path().join("sdk");
    fs::write(&schema, SCHEMA).unwrap();
    fs::write(&profile, PROFILE).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg("sdk")
            .arg(&schema)
            .arg("--profile")
            .arg(&profile)
            .arg("--out-dir")
            .arg(&output)
            .args(extra)
            .output()
            .unwrap()
    };
    assert!(run(&[]).status.success());
    assert!(run(&[]).status.success());
    let client = output.join("src/client.cpp");
    fs::write(&client, "user edit").unwrap();
    assert!(!run(&[]).status.success());
    assert_eq!(fs::read_to_string(&client).unwrap(), "user edit");
    assert!(run(&["--overwrite"]).status.success());
    assert!(!run(&["--overwrite", "--name", "other"]).status.success());
    fs::write(&schema, SCHEMA.replace("string<12>", "string")).unwrap();
    let previous = fs::read(&client).unwrap();
    assert!(!run(&["--overwrite"]).status.success());
    assert_eq!(previous, fs::read(&client).unwrap());
}

#[test]
fn cli_resolves_imports_and_composed_host_profiles() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("product.wl"),
        "version 1; import \"device.wl\";",
    )
    .unwrap();
    fs::write(directory.path().join("device.wl"), SCHEMA).unwrap();
    fs::write(directory.path().join("rpc.bind.wl"), PROFILE).unwrap();
    fs::write(
        directory.path().join("host.bind.wl"),
        "profile version 1; endpoint { rpc_role = client; envelope = native_packet; }",
    )
    .unwrap();
    let output = directory.path().join("sdk");
    let result = Command::new(env!("CARGO_BIN_EXE_wlc"))
        .arg("sdk")
        .arg(directory.path().join("product.wl"))
        .arg("--profile")
        .arg(directory.path().join("rpc.bind.wl"))
        .arg("--profile")
        .arg(directory.path().join("host.bind.wl"))
        .args(["--name", "composed", "--package-version", "2.3.4.dev5"])
        .arg("--out-dir")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        fs::read_to_string(output.join("generated/composed_runtime.h"))
            .unwrap()
            .contains("COMPOSED_RUNTIME_HAS_RPC_SERVER 0")
    );
    assert!(
        fs::read_to_string(output.join("pyproject.toml"))
            .unwrap()
            .contains("version = \"2.3.4.dev5\"")
    );
    assert!(
        fs::read_to_string(output.join("CMakeLists.txt"))
            .unwrap()
            .contains("VERSION 2.3.4")
    );
}
