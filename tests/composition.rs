use std::{fs, process::Command};
use tempfile::tempdir;
use wlc::{analyze_schema, generate_c, load_schema, parse_schema, schema_identity};

#[test]
fn diamond_imports_are_relative_deduplicated_and_wire_equivalent() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("parts")).unwrap();
    for (name, source) in [
        (
            "parts/shared.wl",
            "version 3; enum Mode = 100 { Idle = 0; }",
        ),
        (
            "parts/arm.wl",
            "version 8; import \"shared.wl\"; message Arm = 10 { required Mode mode = 1; }",
        ),
        (
            "upgrade.wl",
            "version 2; import \"parts/shared.wl\"; message Chunk = 20 { required bytes<512> data = 1; }",
        ),
        (
            "product.wl",
            "version 9; import \"parts/arm.wl\"; import \"upgrade.wl\";",
        ),
        (
            "reverse.wl",
            "version 9; import \"upgrade.wl\"; import \"parts/arm.wl\";",
        ),
    ] {
        fs::write(dir.path().join(name), source).unwrap();
    }
    let loaded = load_schema(&dir.path().join("product.wl")).unwrap();
    assert_eq!(loaded.dependencies.len(), 4);
    assert!(loaded.schema.imports.is_empty());
    let model = analyze_schema(&loaded.schema).unwrap();
    let flat = analyze_schema(&parse_schema("version 9; enum Mode = 100 { Idle = 0; } message Arm = 10 { required Mode mode = 1; } message Chunk = 20 { required bytes<512> data = 1; }").unwrap()).unwrap();
    let reverse =
        analyze_schema(&load_schema(&dir.path().join("reverse.wl")).unwrap().schema).unwrap();
    assert_eq!(schema_identity(&model), schema_identity(&flat));
    assert_eq!(schema_identity(&model), schema_identity(&reverse));
    assert_eq!(
        generate_c(&model, "product").unwrap(),
        generate_c(&flat, "product").unwrap()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wlc"))
        .arg("dependencies")
        .arg(dir.path().join("product.wl"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 4);
    // The library cannot silently discard an unresolved import.
    assert!(analyze_schema(&parse_schema("version 1; import \"missing.wl\";").unwrap()).is_err());
}

#[test]
fn invalid_import_graphs_and_global_collisions_are_rejected() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root.wl");
    let child = dir.path().join("child.wl");
    fs::write(&root, "version 1; import \"child.wl\"; message A = 1 {}").unwrap();
    assert!(load_schema(&root).is_err());
    for source in [
        "version 1; import \"root.wl\";",
        "version 1; message B = 1 {}",
        "version 1; message A = 2 {}",
        "version 1; reserved 1;",
        "version 1; message Broken",
    ] {
        fs::write(&child, source).unwrap();
        assert!(load_schema(&root).is_err(), "accepted: {source}");
    }
}
