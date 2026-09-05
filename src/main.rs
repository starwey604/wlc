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
    /// Validate a schema and optionally check it against its predecessor.
    Validate {
        schema: PathBuf,
        /// Check SCHEMA for compatibility with this predecessor.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Resolve and validate PROFILE against SCHEMA.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Generate C and a deterministic manifest; --profile also generates the application runtime.
    Compile {
        schema: PathBuf,
        /// Destination directory for generated artifacts.
        #[arg(long)]
        out_dir: PathBuf,
        /// Check SCHEMA for compatibility with this predecessor.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Resolve PROFILE and generate <module>_runtime.h/.c.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Generate only one profile runtime against separately generated schema artifacts.
    CompileRuntime {
        schema: PathBuf,
        /// Binding profile used to specialize the runtime.
        #[arg(long)]
        profile: PathBuf,
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
        profile: Option<PathBuf>,
    },
}

enum Operation {
    Validate,
    Compile(PathBuf),
    CompileRuntime {
        output: PathBuf,
        runtime_name: Option<String>,
    },
    Identity,
}

fn is_portable_c_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let (schema_path, previous, profile, operation) = match arguments.command {
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
        Command::CompileRuntime {
            schema,
            profile,
            out_dir,
            runtime_name,
            previous,
        } => (
            schema,
            previous,
            Some(profile),
            Operation::CompileRuntime {
                output: out_dir,
                runtime_name,
            },
        ),
        Command::Identity { schema, profile } => (schema, None, profile, Operation::Identity),
    };
    let source = fs::read_to_string(&schema_path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read `{}`", schema_path.display()))?;

    let schema = wlc::parse_schema(&source).map_err(|error| {
        miette::Report::new(error).with_source_code(NamedSource::new(
            schema_path.display().to_string(),
            source.clone(),
        ))
    })?;
    let model = wlc::analyze_schema(&schema).map_err(|error| {
        miette::Report::new(error)
            .with_source_code(NamedSource::new(schema_path.display().to_string(), source))
    })?;
    if let Some(previous) = previous {
        let previous_source = fs::read_to_string(&previous)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not read `{}`", previous.display()))?;
        let previous_schema = wlc::parse_schema(&previous_source).map_err(|error| {
            miette::Report::new(error).with_source_code(NamedSource::new(
                previous.display().to_string(),
                previous_source.clone(),
            ))
        })?;
        let previous_model = wlc::analyze_schema(&previous_schema).map_err(|error| {
            miette::Report::new(error).with_source_code(NamedSource::new(
                previous.display().to_string(),
                previous_source,
            ))
        })?;
        wlc::check_compatibility(&previous_model, &model).map_err(miette::Report::new)?;
    }
    let profile_model = if let Some(profile_path) = &profile {
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
        Some(profile_model)
    } else {
        None
    };
    let identity_operation = matches!(operation, Operation::Identity);
    match operation {
        Operation::Compile(output) => {
            fs::create_dir_all(&output).into_diagnostic()?;
            let stem = schema_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("wirelink_generated");
            let generated = wlc::generate_c(&model, stem).map_err(miette::Report::new)?;
            let generated_runtime = profile_model
                .as_ref()
                .map(|profile| wlc::generate_runtime_c(&model, profile, stem))
                .transpose()
                .map_err(miette::Report::new)?;
            let has_runtime = generated_runtime.is_some();
            let mut artifacts = vec![
                (format!("{stem}.h"), generated.header),
                (format!("{stem}.c"), generated.source),
                (format!("{stem}_bindings.h"), generated.bindings_header),
                (format!("{stem}_bindings.c"), generated.bindings_source),
            ];
            if let Some(generated_runtime) = generated_runtime {
                artifacts.push((format!("{stem}_runtime.h"), generated_runtime.header));
                artifacts.push((format!("{stem}_runtime.c"), generated_runtime.source));
            }
            let manifest_artifacts = artifacts
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
                &manifest_artifacts,
            );
            for (path, contents) in artifacts {
                fs::write(output.join(path), contents).into_diagnostic()?;
            }
            fs::write(output.join(format!("{stem}_manifest.json")), manifest).into_diagnostic()?;
            if has_runtime {
                println!(
                    "generated {}.h/.c, {}_bindings.h/.c, {}_runtime.h/.c, and {}_manifest.json in {}",
                    stem,
                    stem,
                    stem,
                    stem,
                    output.display()
                );
            } else {
                println!(
                    "generated {}.h/.c, {}_bindings.h/.c, and {}_manifest.json in {}",
                    stem,
                    stem,
                    stem,
                    output.display()
                );
            }
        }
        Operation::CompileRuntime {
            output,
            runtime_name,
        } => {
            fs::create_dir_all(&output).into_diagnostic()?;
            let codec_module = schema_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("wirelink_generated");
            let runtime_name = runtime_name.as_deref().unwrap_or(codec_module);
            if !is_portable_c_identifier(runtime_name) {
                return Err(miette::miette!(
                    "runtime name `{runtime_name}` must be a portable C identifier"
                ));
            }
            let profile = profile_model
                .as_ref()
                .expect("compile-runtime always resolves a required profile");
            let generated =
                wlc::generate_runtime_c_named(&model, profile, codec_module, runtime_name)
                    .map_err(miette::Report::new)?;
            let artifacts = [
                (format!("{runtime_name}_runtime.h"), generated.header),
                (format!("{runtime_name}_runtime.c"), generated.source),
            ];
            let manifest_artifacts = artifacts
                .iter()
                .map(|(path, contents)| wlc::ManifestArtifact {
                    path,
                    contents: contents.as_bytes(),
                })
                .collect::<Vec<_>>();
            let manifest = wlc::generate_codegen_manifest(
                runtime_name,
                &model,
                Some(wlc::binding_profile_identity(profile)),
                &manifest_artifacts,
            );
            for (path, contents) in artifacts {
                fs::write(output.join(path), contents).into_diagnostic()?;
            }
            fs::write(
                output.join(format!("{runtime_name}_runtime_manifest.json")),
                manifest,
            )
            .into_diagnostic()?;
            println!(
                "generated {}_runtime.h/.c and {}_runtime_manifest.json against codec module {} in {}",
                runtime_name,
                runtime_name,
                codec_module,
                output.display()
            );
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
    }
    if !identity_operation
        && let (Some(profile_path), Some(profile_model)) = (&profile, &profile_model)
    {
        println!(
            "validated binding profile {} (version {}, {} binding(s))",
            profile_path.display(),
            profile_model.version,
            profile_model.retained_routes.len() + profile_model.rpc_services.len()
        );
    }
    Ok(())
}
