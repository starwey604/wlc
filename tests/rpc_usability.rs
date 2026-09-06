use std::{fs, path::PathBuf, process::Command};
use tempfile::tempdir;

#[test]
fn cache_capacity_and_policy_baseline() {
    let root = std::env::var_os("WIRELINK_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .into()
        });
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("baseline.c"),
        include_str!("fixtures/rpc_cache_baseline.c"),
    )
    .unwrap();
    let binary = directory.path().join("baseline");
    let mut cc = Command::new("cc");
    cc.args([
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Wpedantic",
        "-Werror",
    ]);
    if std::env::var_os("WLC_TEST_SANITIZE").is_some() {
        cc.args([
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ]);
    }
    let output = cc
        .arg("-I")
        .arg(root.join("include"))
        .arg(root.join("src/rpc.c"))
        .arg(directory.path().join("baseline.c"))
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
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
    print!("{}", String::from_utf8_lossy(&output.stdout));
}
