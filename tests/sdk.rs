// SPDX-License-Identifier: Apache-2.0
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::tempdir;
use wlc::{SdkOptions, analyze_schema, generate_sdk, parse_schema};

const SCHEMA: &str = include_str!("fixtures/sdk/device.wl");
const SIMPLE: &str =
    "version 1; message Request @id(1) {} message Response @id(2) {} message Notice @id(3) {}";
const RPC: &str = "node host { call Call(Request) -> Response; }";

fn resolve(
    schema: &str,
    node: &str,
) -> Result<(wlc::SemanticModel, wlc::node::Node), wlc::SdkCodegenError> {
    let ast = parse_schema(&format!("{schema}\n{node}"))
        .map_err(|error| wlc::SdkCodegenError(error.to_string()))?;
    let model = analyze_schema(&ast).map_err(|error| wlc::SdkCodegenError(error.to_string()))?;
    let mut nodes = wlc::node::analyze_nodes(&ast, &model)
        .map_err(|error| wlc::SdkCodegenError(error.to_string()))?;
    assert_eq!(nodes.len(), 1);
    Ok((model, nodes.remove(0)))
}

fn sdk(schema: &str, node: &str) -> Result<wlc::GeneratedSdk, wlc::SdkCodegenError> {
    let (model, node) = resolve(schema, node)?;
    generate_sdk(
        &model,
        &node,
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
fn project_is_deterministic_and_emits_one_external_client_node() {
    let generated = sdk(SCHEMA, "").unwrap();
    assert_eq!(generated, sdk(SCHEMA, "").unwrap());
    assert!(!generated.files["generated/device.c"].contains("fingerprint"));
    assert!(generated.files["generated/device.h"].contains("device_node_t"));
    assert!(generated.files["generated/device.h"].contains("device_call_get_info_ex"));
    assert!(
        generated
            .files
            .keys()
            .all(|path| !path.contains("_kernel."))
    );
    assert!(
        generated.files["src/endpoint.c"].contains("device_init(&owner->endpoint, &owner->port)")
    );
    assert!(!generated.files["src/values.hpp"].contains("has_enabled"));
    assert!(generated.files["src/values.hpp"].contains("has_label"));
    assert!(generated.files.contains_key("python/device_sdk/py.typed"));
    assert!(generated.files["wlc-sdk-manifest.json"].contains("python/device_sdk/__init__.py"));
    assert!(generated.files["cmake/DeviceSdkConfig.cmake.in"].starts_with("@PACKAGE_INIT@"));
    assert!(!generated.files["cmake/DeviceSdkConfig.cmake.in"].contains("RPC_LAYOUT_ENABLE"));
    assert!(!generated.files["README.md"].contains("RPC_LAYOUT_ENABLE"));
    assert!(generated.files["README.md"].contains("DEVICE_BUILD_PYTHON=ON"));
    assert!(generated.files["README.md"].contains("with `ClosedError`"));
}

#[test]
fn rejects_unsupported_nodes_before_emission() {
    for (node, diagnostic) in [
        ("node host { send Request; }", "RPC call routes"),
        (
            "node host { serve Call(Request) -> Response; }",
            "RPC call routes",
        ),
        (
            "node host { framing = cobs_crc16; call Call(Request) -> Response; }",
            "framing",
        ),
        (
            "node host { send Notice; call Call(Request) -> Response; }",
            "RPC-client-only",
        ),
        (
            "node host { recv Notice; call Call(Request) -> Response; }",
            "RPC-client-only",
        ),
        (
            "node host { serve Call(Request) -> Response; call Call; }",
            "RPC-client-only",
        ),
    ] {
        assert!(
            sdk(SIMPLE, node)
                .unwrap_err()
                .to_string()
                .contains(diagnostic)
        );
    }
}

#[test]
fn rejects_unbounded_and_oversized_messages_with_a_diagnostic() {
    for (field, diagnostic) in [
        ("optional string label @id(1);", "finite encoded bound"),
        ("repeated uint32 values @id(1);", "finite encoded bound"),
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
    for (schema, node) in [
        (
            "version 1; enum DeviceSdkCall @id(7) { X = 0; } message Request @id(1) {} message Response @id(2) {}",
            RPC,
        ),
        (SIMPLE, "node host { call Cancel(Request) -> Response; }"),
        (
            "version 1; message Request @id(1) {} message Response @id(2) {} message Notice @id(3) {} message Reply @id(4) {}",
            "node host { call Call(Request) -> Response; call CallSubmit(Notice) -> Reply; }",
        ),
        (
            "version 1; message AsyncClient @id(3) {} message Request @id(1) {} message Response @id(2) {}",
            RPC,
        ),
        (
            "version 1; message Request @id(1) {} message Response @id(2) {} message Notice @id(3) {} message Reply @id(4) {}",
            "node host { call Call(Request) -> Response; call CallAsync(Notice) -> Reply; }",
        ),
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
        (SIMPLE, "node host { call Close(Request) -> Response; }"),
    ] {
        assert!(
            sdk(schema, node)
                .unwrap_err()
                .to_string()
                .contains("collision")
        );
    }
}

#[test]
fn rejects_unsafe_project_options() {
    let (model, node) = resolve(SIMPLE, RPC).unwrap();
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
                &node,
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
                &node,
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
    let sdk = sdk(SCHEMA, "").unwrap();
    write_sdk(&sdk, directory.path());
    fs::write(
        directory.path().join("test.cpp"),
        include_str!("fixtures/sdk/values.cpp"),
    )
    .unwrap();
    fs::write(
        directory.path().join("bridge_test.c"),
        include_str!("fixtures/sdk/bridge.c"),
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
    let mut cc_rt = Command::new("cc");
    cc_rt
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-c",
        ])
        .arg(root.join("src/codec.c"))
        .arg("-I")
        .arg(root.join("include"))
        .arg("-o")
        .arg(directory.path().join("codec_rt.o"));
    let mut core_commands = Vec::new();
    for source in ["endpoint", "packet_port"] {
        let mut command = Command::new("cc");
        command
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-c",
            ])
            .arg(root.join(format!("src/{source}.c")))
            .arg("-I")
            .arg(root.join("include"))
            .arg("-o")
            .arg(directory.path().join(format!("{source}.o")));
        core_commands.push(command);
    }
    let mut cxx = Command::new("c++");
    cxx.args(["-std=c++20", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg(directory.path().join("test.cpp"))
        .arg(directory.path().join("codec.o"))
        .arg(directory.path().join("codec_rt.o"))
        .arg(directory.path().join("endpoint.o"))
        .arg(directory.path().join("packet_port.o"));
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
        for command in [&mut cc, &mut cc_rt, &mut cxx]
            .into_iter()
            .chain(core_commands.iter_mut())
        {
            command.args([
                "-fsanitize=address,undefined",
                "-fno-omit-frame-pointer",
                "-g",
            ]);
        }
    }
    if std::env::var_os("WLC_TEST_X87").is_some() {
        cc.args(["-mfpmath=387", "-mno-sse"]);
        cc_rt.args(["-mfpmath=387", "-mno-sse"]);
        cxx.args(["-mfpmath=387"]);
    }
    // Compile the actual sync/async facade and C-only callback bridge too. This
    // fixture includes names such as Mode and catches C/C++ namespace mistakes.
    let mut bridge = Command::new("cc");
    bridge
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-c",
        ])
        .arg(directory.path().join("src/endpoint.c"))
        .arg("-o")
        .arg(directory.path().join("sdk_endpoint.o"));
    let mut client = Command::new("c++");
    client
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-c",
        ])
        .arg(directory.path().join("src/client.cpp"))
        .arg("-o")
        .arg(directory.path().join("client.o"));
    for command in [&mut bridge, &mut client] {
        for include in [
            root.join("include"),
            root.join("bindings/cpp/include"),
            root.join("runtime/host/include"),
            directory.path().join("include"),
            directory.path().join("generated"),
        ] {
            command.arg("-I").arg(include);
        }
    }
    let bridge_executable = directory.path().join("bridge_test");
    let mut bridge_link = Command::new("cc");
    bridge_link
        .args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg(directory.path().join("bridge_test.c"));
    for object in [
        "codec",
        "codec_rt",
        "endpoint",
        "packet_port",
        "sdk_endpoint",
    ] {
        bridge_link.arg(directory.path().join(format!("{object}.o")));
    }
    for include in [
        root.join("include"),
        root.join("runtime/host/include"),
        directory.path().join("generated"),
        directory.path().join("src"),
    ] {
        bridge_link.arg("-I").arg(include);
    }
    bridge_link.arg("-o").arg(&bridge_executable);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        bridge.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
        bridge_link.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    for command in &mut core_commands {
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{command:?}\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for command in [
        &mut cc,
        &mut cc_rt,
        &mut cxx,
        &mut bridge,
        &mut client,
        &mut bridge_link,
        &mut Command::new(&executable),
        &mut Command::new(&bridge_executable),
    ] {
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
    let output = directory.path().join("sdk");
    fs::write(&schema, SCHEMA).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg("sdk")
            .arg(&schema)
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
fn cli_resolves_imported_rpc_declarations_and_local_host_node() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("product.wl"),
        r#"import "device.wl"; node host { call GetInfo; call Configure; call EchoCall; }"#,
    )
    .unwrap();
    fs::write(
        directory.path().join("device.wl"),
        SCHEMA.split("node host").next().unwrap(),
    )
    .unwrap();
    let output = directory.path().join("sdk");
    let result = Command::new(env!("CARGO_BIN_EXE_wlc"))
        .arg("sdk")
        .arg(directory.path().join("product.wl"))
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
        fs::read_to_string(output.join("generated/composed.h"))
            .unwrap()
            .contains("composed_call_get_info_ex")
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

#[test]
fn udp_storage_covers_both_directions_and_an_oversize_sentinel() {
    let generated = sdk(
        "message Request @id(1) { packed fixed32 values[16] @id(1); } message Response @id(2) {}",
        RPC,
    )
    .unwrap();
    // 64 bytes + tagged key/length + 12-byte RPC framing, then one sentinel.
    let bridge = &generated.files["src/endpoint.c"];
    assert!(bridge.contains("rx[WL_PACKET_RX_SLOTS][79]"));
    assert!(bridge.contains("&owner->port, 78}"));
}

#[test]
fn sdk_bridge_owns_completion_even_with_schema_static_done_symbol() {
    let generated = sdk(
        SIMPLE,
        "node host { call Call(Request) -> Response = application_done; }",
    )
    .unwrap();
    assert!(!generated.files["generated/device.c"].contains("application_done"));
    assert!(generated.files["src/endpoint.c"].contains("device_call_call_ex"));
    assert!(generated.files["src/endpoint.c"].contains("state->request, now, timeout"));
}

#[test]
fn cli_requires_selection_for_multiple_clients_and_synthesizes_only_without_clients() {
    let directory = tempdir().unwrap();
    let schema = directory.path().join("device.wl");
    let output = directory.path().join("sdk");
    fs::write(&schema, format!("{SIMPLE} rpc Call(Request) -> Response; node one {{ call Call; }} node two {{ call Call; }}")).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wlc"))
            .arg("sdk")
            .arg(&schema)
            .arg("--out-dir")
            .arg(&output)
            .args(extra)
            .output()
            .unwrap()
    };
    assert!(!run(&[]).status.success());
    let selected = run(&["--node", "one"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let explicit = fs::read(output.join("generated/device.h")).unwrap();
    fs::write(&schema, format!("{SIMPLE} rpc Call(Request) -> Response;")).unwrap();
    let synthesized = run(&["--overwrite"]);
    assert!(
        synthesized.status.success(),
        "{}",
        String::from_utf8_lossy(&synthesized.stderr)
    );
    assert_eq!(
        explicit,
        fs::read(output.join("generated/device.h")).unwrap()
    );
}
