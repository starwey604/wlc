use std::{fs, path::PathBuf};

use clap::Parser;
use miette::{IntoDiagnostic, NamedSource, Result, WrapErr};

#[derive(Parser)]
#[command(
    version,
    about = "Validate Wirelink schemas and profiles, generate C artifacts, and print diagnostic identities"
)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Generate the static endpoint kernel (alias of compile --profile) (profile version 2).
    CompileKernel {
        schema: PathBuf,
        #[arg(long, required = true)]
        profile: Vec<PathBuf>,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Generate a complete UDP C++ / Python SDK with sync and async clients.
    Sdk {
        schema: PathBuf,
        #[arg(long, required = true)]
        profile: Vec<PathBuf>,
        #[arg(long)]
        out_dir: PathBuf,
        /// C++ namespace; Python package is <name>_sdk. Defaults to the schema stem.
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "0.1.0.dev1")]
        package_version: String,
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Replace generated files in an existing project with the same SDK name.
        #[arg(long)]
        overwrite: bool,
    },
    /// Print the generated-code contract version for build-tool compatibility checks.
    CodegenContract,
    /// Print the canonical C filename/symbol prefix, including keyword escaping.
    ModuleName { schema: PathBuf },
    /// Print canonical transitive schema inputs, one path per line.
    Dependencies { schema: PathBuf },
    /// Validate a schema and optionally check it against its predecessor.
    Validate {
        schema: PathBuf,
        /// Check SCHEMA for compatibility with this predecessor.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Resolve PROFILE against SCHEMA; repeat to compose shared/local bindings.
        #[arg(long)]
        profile: Vec<PathBuf>,
    },
    /// Generate C and a deterministic manifest; --profile generates the static endpoint kernel.
    Compile {
        schema: PathBuf,
        /// Destination directory for generated artifacts.
        #[arg(long)]
        out_dir: PathBuf,
        /// Check SCHEMA for compatibility with this predecessor.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Resolve PROFILE and generate <module>_kernel.h/.c; repeat to compose bindings.
        #[arg(long)]
        profile: Vec<PathBuf>,
    },
    /// Generate only one profile runtime against separately generated schema artifacts.
    CompileRuntime {
        schema: PathBuf,
        /// Binding profiles to compose; duplicates/conflicts are errors.
        #[arg(long, required = true)]
        profile: Vec<PathBuf>,
        /// Destination directory for generated artifacts.
        #[arg(long)]
        out_dir: PathBuf,
        /// Public C prefix and filename stem; defaults to the schema stem.
        #[arg(long)]
        runtime_name: Option<String>,
        /// Check SCHEMA for compatibility with this predecessor.
        #[arg(long)]
        previous: Option<PathBuf>,
    },
    /// Print exact diagnostic identities; not a compatibility or security check.
    Identity {
        schema: PathBuf,
        /// Print the resolved profile identity alongside the schema identity.
        #[arg(long)]
        profile: Vec<PathBuf>,
    },
    /// Print static codec/RPC resources; does not claim unsupported execution paths.
    Plan {
        schema: PathBuf,
        #[arg(long, required = true)]
        profile: Vec<PathBuf>,
    },
}

enum Operation {
    Sdk {
        output: PathBuf,
        name: Option<String>,
        package_version: String,
        overwrite: bool,
    },
    Validate,
    Compile(PathBuf),
    Identity,
    Plan,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let (schema_path, previous, profile, operation) = match arguments.command {
        Command::CompileKernel {
            schema,
            profile,
            out_dir,
        } => (schema, None, profile, Operation::Compile(out_dir)),
        Command::Sdk {
            schema,
            profile,
            out_dir,
            name,
            package_version,
            previous,
            overwrite,
        } => (
            schema,
            previous,
            profile,
            Operation::Sdk {
                output: out_dir,
                name,
                package_version,
                overwrite,
            },
        ),
        Command::Dependencies { schema } => {
            for path in wlc::load_schema(&schema)?.dependencies {
                println!("{}", path.display());
            }
            return Ok(());
        }
        Command::ModuleName { schema } => {
            let stem = schema.file_stem().and_then(|v| v.to_str()).unwrap_or("");
            let module = wlc::codegen::c_module_name(stem);
            if module.is_empty() {
                return Err(miette::miette!(
                    "schema filename has no C identifier characters"
                ));
            }
            println!("{module}");
            return Ok(());
        }
        Command::CodegenContract => {
            println!(
                "{}.{}",
                wlc::CODEGEN_CONTRACT_MAJOR,
                wlc::CODEGEN_CONTRACT_MINOR
            );
            return Ok(());
        }
        Command::Validate {
            schema,
            previous,
            profile,
        } => (schema, previous, profile, Operation::Validate),
        Command::Compile {
            schema,
            out_dir,
            previous,
            profile,
        } => (schema, previous, profile, Operation::Compile(out_dir)),
        Command::CompileRuntime { .. } => {
            return Err(miette::miette!(
                "compile-runtime was removed; migrate profile to version 2 and use compile --profile"
            ));
        }
        Command::Identity { schema, profile } => (schema, None, profile, Operation::Identity),
        Command::Plan { schema, profile } => (schema, None, profile, Operation::Plan),
    };
    let loaded = wlc::load_schema(&schema_path)?;
    let model = wlc::analyze_schema(&loaded.schema).map_err(|error| {
        miette::Report::new(error).with_source_code(NamedSource::new(
            schema_path.display().to_string(),
            loaded.source,
        ))
    })?;
    if let Some(previous) = previous {
        let previous_loaded = wlc::load_schema(&previous)?;
        let previous_model = wlc::analyze_schema(&previous_loaded.schema).map_err(|error| {
            miette::Report::new(error).with_source_code(NamedSource::new(
                previous.display().to_string(),
                previous_loaded.source,
            ))
        })?;
        wlc::check_compatibility(&previous_model, &model).map_err(miette::Report::new)?;
    }
    let mut fragments = Vec::new();
    for profile_path in &profile {
        let profile_source = fs::read_to_string(profile_path)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not read `{}`", profile_path.display()))?;
        let profile = wlc::parse_binding_profile(&profile_source).map_err(|error| {
            miette::Report::new(error).with_source_code(NamedSource::new(
                profile_path.display().to_string(),
                profile_source.clone(),
            ))
        })?;
        let profile_model = wlc::analyze_binding_profile(&profile, &model).map_err(|error| {
            miette::Report::new(error).with_source_code(NamedSource::new(
                profile_path.display().to_string(),
                profile_source,
            ))
        })?;
        fragments.push(profile_model);
    }
    let profile_model = if fragments.is_empty() {
        None
    } else {
        Some(
            wlc::compose_binding_profiles(&fragments)
                .map_err(miette::Report::new)
                .wrap_err_with(|| {
                    format!(
                        "could not compose profiles: {}",
                        profile
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
        )
    };
    let identity_operation = matches!(operation, Operation::Identity | Operation::Plan);
    match operation {
        Operation::Sdk {
            output,
            name,
            package_version,
            overwrite,
        } => {
            let name = name.unwrap_or_else(|| {
                schema_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sdk")
                    .to_owned()
            });
            let generated = wlc::generate_sdk(
                &model,
                profile_model.as_ref().expect("sdk requires profiles"),
                &wlc::SdkOptions {
                    name,
                    package_version,
                },
            )
            .map_err(miette::Report::new)?;
            // Preflight every path before touching an existing project. Invalid
            // schemas/options and accidental edits never cause partial generation.
            if output.join("generated").exists() {
                let entries = fs::read_dir(output.join("generated")).into_diagnostic()?;
                for entry in entries {
                    let path = entry.into_diagnostic()?.path();
                    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
                    if name.ends_with("_runtime.c")
                        || name.ends_with("_endpoint.h")
                        || name.ends_with("_bindings.c")
                    {
                        return Err(miette::miette!(
                            "legacy SDK execution artifacts: generate into a fresh directory and explicitly migrate application files"
                        ));
                    }
                }
            }
            let marker = output.join("wlc-sdk-name.txt");
            if marker.exists()
                && fs::read_to_string(&marker).into_diagnostic()?
                    != generated.files["wlc-sdk-name.txt"]
            {
                return Err(miette::miette!(
                    "SDK name changed: choose a fresh output directory"
                ));
            }
            for (path, contents) in &generated.files {
                let destination = output.join(path);
                if destination.exists() {
                    let existing = fs::read(&destination).into_diagnostic()?;
                    if existing != contents.as_bytes() && !overwrite {
                        return Err(miette::miette!(
                            "refusing to overwrite `{}`; regenerate into a fresh directory or pass --overwrite",
                            destination.display()
                        ));
                    }
                }
            }
            for (path, contents) in &generated.files {
                let destination = output.join(path);
                fs::create_dir_all(destination.parent().expect("project path has parent"))
                    .into_diagnostic()?;
                fs::write(destination, contents).into_diagnostic()?;
            }
            println!(
                "generated C++ / Python SDK ({} files) in {}",
                generated.files.len(),
                output.display()
            );
        }
        Operation::Compile(output) => {
            let stem = schema_path
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("wirelink_generated");
            let module = wlc::codegen::c_module_name(stem);
            let stem = module.as_str();
            let mut files = if let Some(profile) = &profile_model {
                let kernel = wlc::KernelProfile::from_bindings(profile.clone())
                    .map_err(miette::Report::new)?;
                wlc::generate_kernel_c(&model, &kernel, stem)
                    .map_err(miette::Report::new)?
                    .files
            } else {
                let c = wlc::generate_c(&model, stem).map_err(miette::Report::new)?;
                std::collections::BTreeMap::from([
                    (format!("{stem}.h"), c.header),
                    (format!("{stem}_values.h"), c.values_header),
                    (format!("{stem}.c"), c.source),
                ])
            };
            if profile_model.is_none() {
                let artifacts = files
                    .iter()
                    .map(|(path, contents)| wlc::ManifestArtifact {
                        path,
                        contents: contents.as_bytes(),
                    })
                    .collect::<Vec<_>>();
                let manifest = wlc::generate_codegen_manifest(
                    stem,
                    &model,
                    profile_model.as_ref().map(wlc::binding_profile_identity),
                    &artifacts,
                );
                files.insert(format!("{stem}_manifest.json"), manifest);
            }
            // Reject stale legacy execution output rather than leave a second engine.
            for suffix in ["_runtime.c", "_endpoint.h", "_bindings.c", "_advanced.h"] {
                if output.join(format!("{stem}{suffix}")).exists() {
                    return Err(miette::miette!(
                        "legacy execution artifacts in output directory; use a fresh directory"
                    ));
                }
            }
            if profile_model.is_some()
                && !files.contains_key(&format!("{stem}_kernel_transport.c"))
                && output.join(format!("{stem}_kernel_transport.c")).exists()
            {
                return Err(miette::miette!(
                    "stale stream deployment artifacts: use a fresh output directory"
                ));
            }
            fs::create_dir_all(&output).into_diagnostic()?;
            for (name, content) in files {
                fs::write(output.join(name), content).into_diagnostic()?;
            }
            println!("generated current C contract in {}", output.display());
        }
        Operation::Validate => println!(
            "validated {} (version {}, {} declaration(s))",
            schema_path.display(),
            model.version,
            model.declarations.len()
        ),
        Operation::Identity => {
            println!("identity algorithm: {}", wlc::IDENTITY_ALGORITHM);
            println!("schema identity: 0x{:016x}", wlc::schema_identity(&model));
            if let Some(profile_model) = &profile_model {
                println!(
                    "binding profile identity: 0x{:016x}",
                    wlc::binding_profile_identity(profile_model)
                );
            }
        }
        Operation::Plan => {
            print!(
                "{}",
                wlc::plan_manifest::generate_binding_plan_manifest(
                    &model,
                    profile_model.as_ref().expect("plan requires profiles")
                )
                .into_diagnostic()?
            );
        }
    }
    if !identity_operation && let Some(profile_model) = &profile_model {
        println!(
            "validated binding profile {} (version {}, {} binding(s))",
            profile
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            profile_model.version,
            profile_model.retained_routes.len()
                + profile_model.direct_routes.len()
                + profile_model.send_routes.len()
                + profile_model.rpc_services.len()
        );
    }
    Ok(())
}
