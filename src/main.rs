// SPDX-License-Identifier: Apache-2.0
use clap::Parser;
use miette::{IntoDiagnostic, NamedSource, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use wlc::node::{Framing, Node, NodeError, parse_rx_slots, parse_rx_unit_align, select_nodes};
#[derive(Parser)]
#[command(
    version,
    about = "Validate Wirelink schemas and nodes, generate C and host SDKs"
)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}
#[derive(clap::Args)]
struct Selection {
    #[arg(long, conflicts_with = "all_nodes")]
    node: Option<String>,
    #[arg(long)]
    all_nodes: bool,
    /// Override framing on each selected node.
    #[arg(long, value_parser = Framing::parse, value_name = "native|cobs_crc16|length16")]
    framing: Option<Framing>,
    /// Override the number of RX slots.
    #[arg(long, value_parser = parse_rx_slots, value_name = "2|4|8")]
    rx_slots: Option<u8>,
    /// Round each RX slot's byte size up to a multiple of this value.
    #[arg(long, value_parser = parse_rx_unit_align, value_name = "1|64|512")]
    rx_unit_align: Option<u16>,
}
impl Selection {
    fn select(self, all: &[Node]) -> Result<Vec<Node>, NodeError> {
        let mut nodes = select_nodes(all, self.node.as_deref(), self.all_nodes)?;
        for node in &mut nodes {
            node.apply_overrides(self.framing, self.rx_slots, self.rx_unit_align);
        }
        Ok(nodes)
    }
}
#[derive(clap::Subcommand)]
enum Command {
    /// Generate one codec/node C pair and its deterministic manifest.
    Compile {
        schema: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[command(flatten)]
        selection: Selection,
    },
    /// Generate an owning UDP C++20/Python SDK for a client node.
    Sdk {
        schema: PathBuf,
        #[arg(long)]
        node: Option<String>,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "0.1.0.dev1")]
        package_version: String,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
    },
    Validate {
        schema: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
    },
    Identity {
        schema: PathBuf,
    },
    Plan {
        schema: PathBuf,
        #[command(flatten)]
        selection: Selection,
    },
    CodegenContract,
    ModuleName {
        schema: PathBuf,
    },
    Dependencies {
        schema: PathBuf,
    },
}
fn load(path: &Path) -> Result<(wlc::ast::Schema, wlc::SemanticModel)> {
    let loaded = wlc::load_schema(path)?;
    let model = wlc::analyze_schema(&loaded.schema).map_err(|e| {
        miette::Report::new(e)
            .with_source_code(NamedSource::new(path.display().to_string(), loaded.source))
    })?;
    Ok((loaded.schema, model))
}
fn previous(path: Option<PathBuf>, model: &wlc::SemanticModel) -> Result<()> {
    if let Some(path) = path {
        let (_, old) = load(&path)?;
        wlc::check_compatibility(&old, model).map_err(miette::Report::new)?;
    }
    Ok(())
}
fn stem(path: &Path) -> String {
    wlc::codegen::c_module_name(
        path.file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("wirelink_generated"),
    )
}
fn write_files(output: &Path, files: std::collections::BTreeMap<String, String>) -> Result<()> {
    fs::create_dir_all(output).into_diagnostic()?;
    for (name, content) in files {
        let destination = output.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).into_diagnostic()?;
        }
        fs::write(destination, content).into_diagnostic()?;
    }
    Ok(())
}
fn main() -> Result<()> {
    match Arguments::parse().command {
        Command::CodegenContract => println!("{}", wlc::CODEGEN_CONTRACT),
        Command::ModuleName { schema } => {
            let name = stem(&schema);
            if name.is_empty() {
                return Err(miette::miette!(
                    "schema filename has no C identifier characters"
                ));
            }
            println!("{name}");
        }
        Command::Dependencies { schema } => {
            for path in wlc::load_schema(&schema)?.dependencies {
                println!("{}", path.display());
            }
        }
        Command::Validate {
            schema,
            previous: old,
        } => {
            let (ast, model) = load(&schema)?;
            previous(old, &model)?;
            let nodes = wlc::analyze_nodes(&ast, &model).map_err(miette::Report::new)?;
            if nodes.is_empty() {
                wlc::generate_c(&model, &stem(&schema)).map_err(miette::Report::new)?;
            }
            for node in &nodes {
                wlc::generate_node_c(&model, std::slice::from_ref(node), &stem(&schema))
                    .map_err(miette::Report::new)?;
            }
            println!(
                "validated {} (version {}, {} node(s))",
                schema.display(),
                model.version,
                nodes.len()
            );
        }
        Command::Compile {
            schema,
            out_dir,
            previous: old,
            selection,
        } => {
            let (ast, model) = load(&schema)?;
            previous(old, &model)?;
            let all = wlc::analyze_nodes(&ast, &model).map_err(miette::Report::new)?;
            let nodes = selection.select(&all)?;
            let module = stem(&schema);
            let generated =
                wlc::generate_node_c(&model, &nodes, &module).map_err(miette::Report::new)?;
            for suffix in [
                "_runtime.c",
                "_endpoint.h",
                "_bindings.c",
                "_advanced.h",
                "_kernel.h",
                "_kernel.c",
                "_kernel_transport.h",
                "_kernel_transport.c",
                "_kernel_deployment.json",
                "_kernel_manifest.json",
            ] {
                if out_dir.join(format!("{module}{suffix}")).exists() {
                    return Err(miette::miette!(
                        "legacy execution artifacts in output directory; use a fresh directory"
                    ));
                }
            }
            write_files(&out_dir, generated.files)?;
            println!("generated current C contract in {}", out_dir.display());
        }
        Command::Identity { schema } => {
            let (ast, model) = load(&schema)?;
            let nodes = wlc::analyze_nodes(&ast, &model).map_err(miette::Report::new)?;
            println!(
                "identity algorithm: {}\nschema identity: 0x{:016x}",
                wlc::IDENTITY_ALGORITHM,
                wlc::schema_identity(&model)
            );
            for node in &nodes {
                println!(
                    "node {} identity: 0x{:016x}",
                    node.name,
                    wlc::node_identity(node)
                );
            }
        }
        Command::Plan { schema, selection } => {
            let (ast, model) = load(&schema)?;
            let all = wlc::analyze_nodes(&ast, &model).map_err(miette::Report::new)?;
            let nodes = selection.select(&all)?;
            print!(
                "{}",
                wlc::plan_manifest::generate_node_plan_manifest(&model, &nodes)
                    .map_err(miette::Report::new)?
            );
        }
        Command::Sdk {
            schema,
            node,
            out_dir,
            name,
            package_version,
            previous: old,
            overwrite,
        } => {
            let (ast, model) = load(&schema)?;
            previous(old, &model)?;
            let nodes = wlc::analyze_nodes(&ast, &model).map_err(miette::Report::new)?;
            let node = wlc::node::sdk_node(&ast, &model, &nodes, node.as_deref())
                .map_err(miette::Report::new)?;
            let generated = wlc::generate_sdk(
                &model,
                &node,
                &wlc::SdkOptions {
                    name: name.unwrap_or_else(|| stem(&schema)),
                    package_version,
                },
            )
            .map_err(miette::Report::new)?;
            let marker = out_dir.join("wlc-sdk-name.txt");
            if marker.exists()
                && fs::read_to_string(marker).into_diagnostic()?
                    != generated.files["wlc-sdk-name.txt"]
            {
                return Err(miette::miette!(
                    "SDK name changed: choose a fresh output directory"
                ));
            }
            if out_dir.join("generated").exists() {
                for entry in fs::read_dir(out_dir.join("generated")).into_diagnostic()? {
                    let path = entry.into_diagnostic()?.path();
                    let file = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
                    if file.contains("_kernel")
                        || file.ends_with("_runtime.c")
                        || file.ends_with("_endpoint.h")
                        || file.ends_with("_bindings.c")
                    {
                        return Err(miette::miette!(
                            "legacy SDK execution artifacts: generate into a fresh directory"
                        ));
                    }
                }
            }
            for (path, content) in &generated.files {
                let path = out_dir.join(path);
                if path.exists()
                    && fs::read(&path).into_diagnostic()? != content.as_bytes()
                    && !overwrite
                {
                    return Err(miette::miette!(
                        "refusing to overwrite `{}`; use --overwrite",
                        path.display()
                    ));
                }
            }
            let count = generated.files.len();
            write_files(&out_dir, generated.files)?;
            println!(
                "generated C++ / Python SDK ({count} files) in {}",
                out_dir.display()
            );
        }
    }
    Ok(())
}
