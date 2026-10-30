// SPDX-License-Identifier: Apache-2.0
//! Real archive extraction, ELF GC and user placement across GCC/Clang and LTO.
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c,
    generate_runtime_c_named, parse_binding_profile, parse_schema,
};

#[test]
fn rpc_layout_configuration_preserves_gc_and_user_placement() {
    let schema = analyze_schema(&parse_schema("version 1; message Request @id(1) { required uint32 value @id(1); } message Response @id(2) { required uint32 value @id(1); }").unwrap()).unwrap();
    let profile = analyze_binding_profile(
        &parse_binding_profile(
            "profile version 1; rpc Query { request = Request; response = Response; }",
        )
        .unwrap(),
        &schema,
    )
    .unwrap();
    let codec = generate_c(&schema, "task").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "task").unwrap();
    let keyword = generate_runtime_c_named(&schema, &profile, "task", "switch_").unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let directory = std::env::var_os("WLC_ELF_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().into());
    fs::create_dir_all(&directory).unwrap();
    for (name, source) in [
        ("task.h", codec.header),
        ("task.c", codec.source),
        ("task_values.h", codec.values_header),
        ("task_bindings.h", codec.bindings_header),
        ("task_bindings.c", codec.bindings_source),
        ("task_runtime.h", runtime.header),
        ("task_runtime.c", runtime.source),
        ("switch__runtime.h", keyword.header),
        ("switch__runtime.c", keyword.source),
    ] {
        fs::write(directory.join(name), source).unwrap();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let output = Command::new("python3")
        .arg(root.join("wlc/tests/fixtures/rpc_layout.py"))
        .arg(&root)
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn rpc_layout_configuration_names_are_reserved() {
    let model = analyze_schema(
        &parse_schema("version 1; message Request @id(1) {} message Response @id(2) {}").unwrap(),
    )
    .unwrap();
    let binding = analyze_binding_profile(
        &parse_binding_profile(
            "profile version 1; rpc Query { request = Request; response = Response; }",
        )
        .unwrap(),
        &model,
    )
    .unwrap();
    assert!(
        generate_runtime_c(&model, &binding, "wlc")
            .unwrap_err()
            .0
            .contains("global RPC layout configuration")
    );
    for name in [
        "WLC_RPC_LAYOUT_ENABLE",
        "TASK_RPC_LAYOUT_ENABLE",
        "TASK_RPC_LAYOUT",
        "TASK_RPC_LAYOUT_task_runtime_service",
        "WLC_TASK_RPC_LAYOUT_ACTIVE",
    ] {
        let schema = analyze_schema(&parse_schema(&format!("version 1; enum Names @id(1) {{ {name} = 0; }} message Request @id(2) {{}} message Response @id(3) {{}} ")).unwrap()).unwrap();
        let profile = analyze_binding_profile(
            &parse_binding_profile(
                "profile version 1; rpc Query { request = Request; response = Response; }",
            )
            .unwrap(),
            &schema,
        )
        .unwrap();
        let error = generate_runtime_c(&schema, &profile, "task").unwrap_err();
        assert!(error.0.contains("reserved RPC layout"), "{error}");
    }
}
