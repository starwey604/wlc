use std::{fs, path::PathBuf};

use clap::Parser;
use miette::{IntoDiagnostic, NamedSource, Result, WrapErr};

#[derive(Parser)]
#[command(
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
    /// Generate codec and typed-binding C; --profile also generates the application runtime.
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
    Identity,
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
            fs::write(output.join(format!("{stem}.h")), generated.header).into_diagnostic()?;
            fs::write(output.join(format!("{stem}.c")), generated.source).into_diagnostic()?;
            fs::write(
                output.join(format!("{stem}_bindings.h")),
                generated.bindings_header,
            )
            .into_diagnostic()?;
            fs::write(
                output.join(format!("{stem}_bindings.c")),
                generated.bindings_source,
            )
            .into_diagnostic()?;
            if let Some(generated_runtime) = generated_runtime {
                fs::write(
                    output.join(format!("{stem}_runtime.h")),
                    generated_runtime.header,
                )
                .into_diagnostic()?;
                fs::write(
                    output.join(format!("{stem}_runtime.c")),
                    generated_runtime.source,
                )
                .into_diagnostic()?;
                println!(
                    "generated {}.h/.c, {}_bindings.h/.c, and {}_runtime.h/.c in {}",
                    stem,
                    stem,
                    stem,
                    output.display()
                );
            } else {
                println!(
                    "generated {}.h/.c and {}_bindings.h/.c in {}",
                    stem,
                    stem,
                    output.display()
                );
            }
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
