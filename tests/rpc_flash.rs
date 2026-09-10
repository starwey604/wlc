// SPDX-License-Identifier: Apache-2.0
//! Guard generated RPC text growth at -O2, independently of -Os and LTO.
use std::{fs, path::PathBuf, process::Command};
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

#[test]
fn many_managed_services_share_control_flow_at_o2() {
    if Command::new("arm-none-eabi-gcc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping Cortex-M flash gate: arm-none-eabi-gcc is not installed");
        return;
    }
    let mut source = String::from("version 1;\n");
    let mut binding = String::from("profile version 1; endpoint { rpc_role = server; }\n");
    for i in 0..24 {
        source.push_str(&format!("message Request{i} @id({}) {{ required uint32 input @id(1); }}\nmessage Response{i} @id({}) {{ required uint32 output @id(1); }}\n", 2 * i + 1, 2 * i + 2));
        binding.push_str(&format!(
            "rpc Operation{i} {{ request = Request{i}; response = Response{i}; }}\n"
        ));
    }
    let schema = analyze_schema(&parse_schema(&source).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(&binding).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, "flash").unwrap();
    let runtime = generate_runtime_c(&schema, &profile, "flash").unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("flash.h", codec.header),
        ("flash_values.h", codec.values_header),
        ("flash_bindings.h", codec.bindings_header),
        ("flash_runtime.h", runtime.header),
        ("flash_runtime.c", runtime.source),
    ] {
        fs::write(directory.path().join(name), text).unwrap();
    }
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let object = directory.path().join("runtime.o");
    let output = Command::new("arm-none-eabi-gcc")
        .args([
            "-std=c11",
            "-mcpu=cortex-m7",
            "-mthumb",
            "-O2",
            "-ffunction-sections",
            "-fdata-sections",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(root.join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg("-c")
        .arg(directory.path().join("flash_runtime.c"))
        .arg("-o")
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("arm-none-eabi-nm")
        .args(["-S", "--size-sort"])
        .arg(&object)
        .output()
        .unwrap();
    assert!(output.status.success());
    let symbols = String::from_utf8(output.stdout).unwrap();
    // Count the dispatch AND extracted helpers/typed completion adapters, so
    // moving text out of the dispatcher alone cannot satisfy this budget.
    let size: usize = symbols
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let name = *fields.last()?;
            (name.contains("runtime_dispatch_event")
                || name.contains("rpc_request_")
                || name.contains("rpc_finish_response")
                || name.contains("server_finish")
                || name.contains("encode_response")
                || name.contains("server_complete")
                || name.contains("server_reject"))
            .then(|| usize::from_str_radix(fields.get(1)?, 16).ok())
            .flatten()
        })
        .sum();
    eprintln!("24 managed RPCs, Cortex-M7 -O2 dispatch/completion text: {size} B");
    assert!(size > 1000, "symbol accounting failed: {symbols}");
    assert!(
        size <= 24576,
        "RPC text regressed: {size} B > 24 KiB\n{symbols}"
    );
}
