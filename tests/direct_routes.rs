use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, compose_binding_profiles, generate_c,
    generate_runtime_c_named, parse_binding_profile, parse_schema,
};

const SCHEMA: &str = "version 1; message Chunk = 20 { required bytes<512> data = 1; } message Repeated = 21 { repeated uint32 items = 1; }";

#[test]
fn direct_validation_and_generated_names_are_checked() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let resolve = |text: &str| {
        analyze_binding_profile(
            &parse_binding_profile(&format!("profile version 1; {text}")).unwrap(),
            &schema,
        )
    };
    let direct = resolve("direct Chunk { delivery = reliable; }").unwrap();
    assert!(compose_binding_profiles(&[direct.clone(), direct]).is_err());
    for text in [
        "direct Repeated { delivery = reliable; }",
        "direct Missing { delivery = reliable; }",
        "direct Chunk { delivery = reliable; } direct Chunk { delivery = reliable; }",
    ] {
        assert!(resolve(text).is_err());
    }
    let schema = analyze_schema(
        &parse_schema("version 1; message Chunk = 20 {} message Reply = 22 {}").unwrap(),
    )
    .unwrap();
    let resolve = |text: &str| {
        analyze_binding_profile(
            &parse_binding_profile(&format!("profile version 1; {text}")).unwrap(),
            &schema,
        )
        .unwrap()
    };
    assert!(
        compose_binding_profiles(&[
            resolve("direct Chunk { delivery = reliable; }"),
            resolve("latest Chunk { delivery = reliable; }")
        ])
        .is_err()
    );
    assert!(
        compose_binding_profiles(&[
            resolve("direct Chunk { delivery = reliable; }"),
            resolve("rpc Query { request = Chunk; response = Reply; }")
        ])
        .is_err()
    );
    let schema = analyze_schema(
        &parse_schema(
            "version 1; message Chunk = 20 {} message Request = 21 {} message Response = 22 {}",
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(&parse_binding_profile("profile version 1; direct Chunk { delivery = reliable; } rpc chunk { request = Request; response = Response; }").unwrap(), &schema).unwrap();
    assert!(generate_runtime_c_named(&schema, &profile, "demo", "receiver").is_err());
}

#[test]
fn direct_borrowed_routes_compile_run_and_release_all_paths() {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let temp = tempdir().unwrap();
    let codec = generate_c(&schema, "demo").unwrap();
    for (name, text) in [
        ("demo.h", codec.header),
        ("demo.c", codec.source),
        ("demo_values.h", codec.values_header),
        ("demo_bindings.h", codec.bindings_header),
        ("demo_bindings.c", codec.bindings_source),
    ] {
        fs::write(temp.path().join(name), text).unwrap();
    }
    for (name, route) in [("sender", "send"), ("receiver", "direct")] {
        let profile = analyze_binding_profile(
            &parse_binding_profile(&format!(
                "profile version 1; {route} Chunk {{ delivery = reliable; }}"
            ))
            .unwrap(),
            &schema,
        )
        .unwrap();
        let generated = generate_runtime_c_named(&schema, &profile, "demo", name).unwrap();
        for (suffix, text) in [
            ("runtime.h", generated.header),
            ("runtime.c", generated.source),
            ("endpoint.h", generated.endpoint_header),
        ] {
            fs::write(temp.path().join(format!("{name}_{suffix}")), text).unwrap();
        }
    }
    fs::write(
        temp.path().join("test.c"),
        include_str!("fixtures/direct_routes.c"),
    )
    .unwrap();
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let mut cc = Command::new("cc");
    cc.args(["-std=c11", "-Wall", "-Wextra", "-Wpedantic", "-Werror"])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(root.join("tests/support"))
        .arg("-I")
        .arg(temp.path());
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "c") {
            cc.arg(path);
        }
    }
    cc.arg(root.join("adapters/loopback/src/loopback.c"));
    for file in [
        "demo.c",
        "demo_bindings.c",
        "sender_runtime.c",
        "receiver_runtime.c",
        "test.c",
    ] {
        cc.arg(temp.path().join(file));
    }
    let binary = temp.path().join("test");
    let output = cc.arg("-o").arg(&binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(
        temp.path().join("headers.cpp"),
        "#include \"receiver_endpoint.h\"\n",
    )
    .unwrap();
    let output = Command::new("c++")
        .args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(temp.path())
        .arg(temp.path().join("headers.cpp"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
