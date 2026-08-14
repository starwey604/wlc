use std::{fs, path::PathBuf};

use clap::Parser;
use miette::{IntoDiagnostic, NamedSource, Result, WrapErr};

#[derive(Parser)]
#[command(about = "Validate Wirelink schema files")]
struct Arguments {
    /// Schema file to validate.
    schema: PathBuf,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let source = fs::read_to_string(&arguments.schema)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read `{}`", arguments.schema.display()))?;

    let schema = wlc::parse_schema(&source).map_err(|error| {
        miette::Report::new(error).with_source_code(NamedSource::new(
            arguments.schema.display().to_string(),
            source,
        ))
    })?;
    println!(
        "validated {} (version {}, {} declaration(s))",
        arguments.schema.display(),
        schema.version.value,
        schema.declarations.len()
    );
    Ok(())
}
