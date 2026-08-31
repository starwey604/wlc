use std::{fs, path::PathBuf};

use clap::Parser;
use miette::{IntoDiagnostic, NamedSource, Result, WrapErr};

#[derive(Parser)]
#[command(about = "Validate and compile Wirelink schema files")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Validate a schema and optionally check it against its predecessor.
    Validate {
        schema: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Validate an optional application binding-profile sidecar.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Generate standalone C codecs and optional Wirelink-core bindings.
    Compile {
        schema: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Validate an optional application binding-profile sidecar.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let (schema_path, previous, profile, output) = match arguments.command {
        Command::Validate {
            schema,
            previous,
            profile,
        } => (schema, previous, profile, None),
        Command::Compile {
            schema,
            out_dir,
            previous,
            profile,
        } => (schema, previous, profile, Some(out_dir)),
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
    if let Some(output) = output {
        fs::create_dir_all(&output).into_diagnostic()?;
        let stem = schema_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("wirelink_generated");
        let generated = wlc::generate_c(&model, stem).map_err(miette::Report::new)?;
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
        println!(
            "generated {}.h/.c and {}_bindings.h/.c in {}",
            stem,
            stem,
            output.display()
        );
    } else {
        println!(
            "validated {} (version {}, {} declaration(s))",
            schema_path.display(),
            model.version,
            model.declarations.len()
        );
    }
    if let (Some(profile_path), Some(profile_model)) = (&profile, &profile_model) {
        println!(
            "validated binding profile {} (version {}, {} binding(s))",
            profile_path.display(),
            profile_model.version,
            profile_model.retained_routes.len() + profile_model.rpc_services.len()
        );
    }
    Ok(())
}
