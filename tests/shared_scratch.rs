use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn decode_detail_names_cannot_collide_with_schema_types() {
    let schema = analyze_schema(
        &parse_schema(
            r#"
version 1;
message Req @id(1) {}
message Res @id(2) {}
message DemoRuntimeSmallDecodeDetail @id(3) {}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile("profile version 1; rpc Small { request = Req; response = Res; }")
            .unwrap(),
        &schema,
    )
    .unwrap();
    assert!(generate_runtime_c(&schema, &profile, "demo").is_err());
}

#[test]
fn unbounded_repeated_decode_configuration_stays_per_service() {
    let schema = analyze_schema(
        &parse_schema(
            r#"
version 1;
message Nested @id(1) { repeated int32 values @id(1); }
message FirstRequest @id(2) { required Nested nested @id(1); }
message FirstResponse @id(3) {}
message SecondRequest @id(4) { repeated int32 values @id(1); }
message SecondResponse @id(5) {}
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(
            r#"
profile version 1;
rpc First { request = FirstRequest; response = FirstResponse; }
rpc Second { request = SecondRequest; response = SecondResponse; }
"#,
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let codec = generate_c(&schema, "demo").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
    let directory = tempdir().unwrap();
    for (name, contents) in [
        ("demo.h", codec.header),
        ("demo_values.h", codec.values_header),
        ("demo_bindings.h", codec.bindings_header),
        ("demo_runtime.h", runtime.header),
    ] {
        fs::write(directory.path().join(name), contents).unwrap();
    }
    fs::write(directory.path().join("headers.c"), "#include \"demo_runtime.h\"\n#include <stddef.h>\n_Static_assert(offsetof(demo_runtime_instance_t, first_scratch) != offsetof(demo_runtime_instance_t, second_scratch), \"caller-configured backing pointers must not alias across services\");\n").unwrap();
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let output = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
        ])
        .arg("-I")
        .arg(root.join("include"))
        .arg(directory.path().join("headers.c"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shared_scratch_uses_maximum_capacity_and_preserves_fingerprints() {
    let schema = analyze_schema(
        &parse_schema(
            r#"
version 1;
message SmallRequest @id(1) { required string<17> name @id(1); }
message SmallResponse @id(2) {}
message LargeRequest @id(3) { required packed float64 values[32] @id(1); }
message LargeResponse @id(4) { required bytes<512> data @id(1); }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(
            r#"
profile version 1;
rpc Small { request = SmallRequest; response = SmallResponse; }
rpc Large { request = LargeRequest; response = LargeResponse; }
"#,
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let codec = generate_c(&schema, "demo").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
    let directory = tempdir().unwrap();
    for (name, contents) in [
        ("demo.h", codec.header),
        ("demo_values.h", codec.values_header),
        ("demo.c", codec.source),
        ("demo_bindings.h", codec.bindings_header),
        ("demo_bindings.c", codec.bindings_source),
        ("demo_runtime.h", runtime.header),
        ("demo_runtime.c", runtime.source),
        ("test.c", include_str!("fixtures/shared_scratch.c").into()),
    ] {
        fs::write(directory.path().join(name), contents).unwrap();
    }
    // GCC accepts nested anonymous type declarations as an extension; enforce
    // the actual C++ rule even where Clang is not installed.
    let header = fs::read_to_string(directory.path().join("demo_runtime.h")).unwrap();
    assert!(header.contains("demo_runtime_small_decode_detail_t small_scratch;"));
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    fs::write(directory.path().join("consumer.cpp"), "#include \"demo_runtime.h\"\n#include <cstddef>\nstatic_assert(offsetof(demo_runtime_instance_t, small_scratch) == offsetof(demo_runtime_instance_t, large_scratch));\n").unwrap();
    for compiler in ["c++", "clang++"] {
        let result = Command::new(compiler)
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
            .arg(directory.path().join("consumer.cpp"))
            .output();
        if compiler == "clang++"
            && result
                .as_ref()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        let output = result.unwrap();
        assert!(
            output.status.success(),
            "{compiler}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ])
    .arg("-I")
    .arg(root.join("include"))
    .arg("-I")
    .arg(directory.path());
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let mut sources = fs::read_dir(root.join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect::<Vec<_>>();
    sources.sort();
    cc.args(sources)
        .arg(root.join("adapters/loopback/src/loopback.c"));
    for name in ["demo.c", "demo_bindings.c", "test.c"] {
        cc.arg(directory.path().join(name));
    }
    let executable = directory.path().join("shared-scratch");
    let output = cc.arg("-o").arg(&executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
