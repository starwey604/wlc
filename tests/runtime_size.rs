use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use wlc::{
    analyze_binding_profile, analyze_schema, generate_c, generate_runtime_c, parse_binding_profile,
    parse_schema,
};

const SCHEMA: &str = r#"
version 1;
enum Status = 1 { OK = 0; FAILED = 1; }
message State = 2 { optional uint32 sequence = 1; }
message Alarm = 3 { optional uint32 code = 1; }
message Request = 4 { optional uint32 operation_id = 1; optional bytes body = 2; }
message Response = 5 { optional uint32 operation_id = 1; optional Status status = 2; }
"#;

const PROFILE: &str = r#"
profile version 1;
latest State { delivery = unreliable; }
fifo Alarm { delivery = reliable; }
rpc Execute {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#;

const RETAINED_PROFILE: &str = r#"
profile version 1;
latest State { delivery = unreliable; }
fifo Alarm { delivery = reliable; }
"#;

const RPC_PROFILE: &str = r#"
profile version 1;
rpc Execute {
  request = Request;
  response = Response;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = status;
  request_delivery = reliable;
  response_delivery = reliable;
}
"#;

const HOT_PATH_TEXT_BUDGET: usize = 2944;
const ASSEMBLY_TEXT_BUDGET: usize = 1200;
const COMBINED_TEXT_BUDGET: usize = 4096;

fn wirelink_root() -> std::path::PathBuf {
    fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("include"),
    )
    .unwrap()
    .parent()
    .unwrap()
    .to_path_buf()
}

fn write_fixture(directory: &Path, profile_source: &str, module: &str) {
    let schema = analyze_schema(&parse_schema(SCHEMA).unwrap()).unwrap();
    let profile =
        analyze_binding_profile(&parse_binding_profile(profile_source).unwrap(), &schema).unwrap();
    let codec = generate_c(&schema, module).unwrap();
    let runtime = generate_runtime_c(&schema, &profile, module).unwrap();
    for (name, contents) in [
        (format!("{module}.h"), codec.header),
        (format!("{module}_bindings.h"), codec.bindings_header),
        (format!("{module}_runtime.h"), runtime.header),
        (format!("{module}_runtime.c"), runtime.source),
    ] {
        fs::write(directory.join(name), contents).unwrap();
    }
}

fn assert_host_layout(profile: &str, module: &str, assertions: &str) {
    let directory = tempdir().unwrap();
    write_fixture(directory.path(), profile, module);
    fs::write(
        directory.path().join("sizes.c"),
        format!(
            "#include \"{module}_runtime.h\"\n\n{assertions}\nint main(void) {{ return 0; }}\n"
        ),
    )
    .unwrap();

    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(wirelink_root().join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg(directory.path().join("sizes.c"))
        .status()
        .unwrap();
    assert!(status.success(), "runtime size assertions must compile");
}

#[test]
fn combined_runtime_result_has_bounded_host_layout() {
    assert_host_layout(
        PROFILE,
        "runtime_size",
        r#"_Static_assert(RUNTIME_SIZE_RUNTIME_CODEGEN_ABI_VERSION == 4U,
               "unexpected generated ABI");
_Static_assert(sizeof(runtime_size_runtime_retained_detail_t) <= 12U,
               "retained detail regressed");
_Static_assert(sizeof(runtime_size_runtime_rpc_detail_t) <= 80U,
               "RPC detail regressed");
_Static_assert(sizeof(runtime_size_runtime_result_t) <= 96U,
               "combined runtime result regressed");"#,
    );
}

#[test]
fn retained_only_result_elides_rpc_layout() {
    assert_host_layout(
        RETAINED_PROFILE,
        "retained_size",
        r#"_Static_assert(RETAINED_SIZE_RUNTIME_CODEGEN_ABI_VERSION == 4U,
               "unexpected generated ABI");
_Static_assert(sizeof(retained_size_runtime_retained_detail_t) <= 12U,
               "retained detail regressed");
_Static_assert(sizeof(retained_size_runtime_result_t) <= 24U,
               "retained-only runtime result regressed");"#,
    );
}

#[test]
fn rpc_only_result_has_bounded_layout() {
    assert_host_layout(
        RPC_PROFILE,
        "rpc_size",
        r#"_Static_assert(RPC_SIZE_RUNTIME_CODEGEN_ABI_VERSION == 4U,
               "unexpected generated ABI");
_Static_assert(sizeof(rpc_size_runtime_rpc_detail_t) <= 80U,
               "RPC detail regressed");
_Static_assert(sizeof(rpc_size_runtime_result_t) <= 96U,
               "RPC-only runtime result regressed");"#,
    );
}

#[test]
fn combined_runtime_cortex_m_text_stays_within_split_budgets_when_toolchain_exists() {
    if Command::new("arm-none-eabi-gcc")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let directory = tempdir().unwrap();
    write_fixture(directory.path(), PROFILE, "runtime_size");
    let object = directory.path().join("runtime.o");
    let compile = Command::new("arm-none-eabi-gcc")
        .args([
            "-std=c11",
            "-mcpu=cortex-m4",
            "-mthumb",
            "-Os",
            "-ffunction-sections",
            "-fdata-sections",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-I",
        ])
        .arg(wirelink_root().join("include"))
        .arg("-I")
        .arg(directory.path())
        .arg("-c")
        .arg(directory.path().join("runtime_size_runtime.c"))
        .arg("-o")
        .arg(&object)
        .status()
        .unwrap();
    assert!(compile.success(), "Cortex-M runtime object must compile");

    let size = Command::new("arm-none-eabi-size")
        .arg(&object)
        .output()
        .unwrap();
    assert!(size.status.success());
    let stdout = String::from_utf8(size.stdout).unwrap();
    let symbols = Command::new("arm-none-eabi-nm")
        .args(["-S", "--size-sort"])
        .arg(&object)
        .output()
        .unwrap();
    assert!(symbols.status.success());
    let symbols = String::from_utf8(symbols.stdout).unwrap();
    let text_size = stdout
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<usize>().ok())
        .expect("GNU size text column");
    let assembly_text = symbols
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            let name = columns.last()?;
            let is_assembly = [
                "runtime_size_runtime_storage_region",
                "runtime_size_runtime_layout",
                "runtime_size_runtime_requirements",
                "runtime_size_runtime_init",
            ]
            .iter()
            .any(|needle| name.contains(needle));
            is_assembly
                .then(|| usize::from_str_radix(columns.get(1)?, 16).ok())
                .flatten()
        })
        .sum::<usize>();
    let hot_path_text = text_size
        .checked_sub(assembly_text)
        .expect("assembly symbols cannot exceed total text");

    assert!(
        hot_path_text <= HOT_PATH_TEXT_BUDGET,
        "generated runtime hot path is {hot_path_text} bytes, budget is {HOT_PATH_TEXT_BUDGET}\n{stdout}\n{symbols}"
    );
    assert!(
        assembly_text <= ASSEMBLY_TEXT_BUDGET,
        "generated runtime assembly is {assembly_text} bytes, budget is {ASSEMBLY_TEXT_BUDGET}\n{stdout}\n{symbols}"
    );
    assert!(
        text_size <= COMBINED_TEXT_BUDGET,
        "combined generated runtime .text is {text_size} bytes, budget is {COMBINED_TEXT_BUDGET}\n{stdout}\n{symbols}"
    );
}
