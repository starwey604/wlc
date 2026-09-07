use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn ordinary_async_endpoints_own_values_and_recycle_calls() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let schema = analyze_schema(
        &parse_schema(
            r#"
version 1;
message Request @id(2) { required int32 input @id(1); required string<31> name @id(2); }
message Response @id(3) { required int32 output @id(1); required string<31> name @id(2); }
message Empty @id(4) {}
message Large @id(5) { required bytes<2023> data @id(1); }
"#,
        )
        .unwrap(),
    )
    .unwrap();
    for request in ["reliable", "unreliable"] {
        for response in ["reliable", "unreliable"] {
            let profile = format!(
                "profile version 1; rpc Execute {{ request = Request; response = Response; request_delivery = {request}; response_delivery = {response}; }} rpc Download {{ request = Empty; response = Large; }}"
            );
            let profile =
                analyze_binding_profile(&parse_binding_profile(&profile).unwrap(), &schema)
                    .unwrap();
            let codec = generate_c(&schema, "demo").unwrap();
            let runtime = generate_runtime_c(&schema, &profile, "demo").unwrap();
            let directory = tempdir().unwrap();
            for (name, text) in [
                ("demo.h", codec.header),
                ("demo_values.h", codec.values_header),
                ("demo.c", codec.source),
                ("demo_bindings.h", codec.bindings_header),
                ("demo_bindings.c", codec.bindings_source),
                ("demo_runtime.h", runtime.header),
                ("demo_endpoint.h", runtime.endpoint_header),
                ("demo_runtime.c", runtime.source),
                ("test.c", include_str!("fixtures/async_endpoint.c").into()),
                (
                    "sync_endpoint.c",
                    include_str!("fixtures/sync_endpoint.c").into(),
                ),
                (
                    "allocated_endpoint.c",
                    include_str!("fixtures/allocated_endpoint.c").into(),
                ),
            ] {
                fs::write(directory.path().join(name), text).unwrap();
            }
            fs::write(directory.path().join("headers.cpp"), "#include \"demo_endpoint.h\"\nstatic demo_endpoint_t endpoint;\nstatic void done(void *, const wl_rpc_completion_t *, const response_value_t *) {}\nint main() { request_value_t request{}; return demo_endpoint_execute_async(&endpoint, &request, 100, done, nullptr, nullptr); }\n").unwrap();
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
                .arg(root.join("tests/support"))
                .arg("-I")
                .arg(directory.path())
                .arg(directory.path().join("headers.cpp"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            for capacity in [1, 4] {
                let mut cc = Command::new("cc");
                cc.args([
                    "-std=c11",
                    "-O2",
                    "-Wall",
                    "-Wextra",
                    "-Wpedantic",
                    "-Werror",
                ])
                .arg(format!("-DDEMO_ENDPOINT_RPC_CAPACITY={capacity}"))
                .arg("-I")
                .arg(root.join("include"))
                .arg("-I")
                .arg(root.join("tests/support"))
                .arg("-I")
                .arg(root.join("runtime/storage/include"))
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
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
                    .collect::<Vec<_>>();
                sources.sort();
                cc.args(sources)
                    .arg(root.join("runtime/storage/src/fixed_pool.c"))
                    .arg(root.join("adapters/loopback/src/loopback.c"));
                for file in ["demo.c", "demo_bindings.c", "demo_runtime.c", "test.c"] {
                    cc.arg(directory.path().join(file));
                }
                let binary = directory.path().join("async-test");
                let output = cc.arg("-o").arg(&binary).output().unwrap();
                assert!(
                    output.status.success(),
                    "{request}/{response}/{capacity}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let output = Command::new(binary).output().unwrap();
                assert!(
                    output.status.success(),
                    "{request}/{response}/{capacity}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
        }
    }
}
