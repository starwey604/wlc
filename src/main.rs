use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let Some(path) = arguments.next() else {
        eprintln!("usage: {} <schema.wl>", program.to_string_lossy());
        return ExitCode::from(2);
    };

    if arguments.next().is_some() {
        eprintln!("usage: {} <schema.wl>", program.to_string_lossy());
        return ExitCode::from(2);
    }

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: {error}", path.to_string_lossy());
            return ExitCode::from(2);
        }
    };

    match wlc::parse_schema(&source) {
        Ok(schema) => {
            println!(
                "validated {} (version {}, {} declaration(s))",
                path.to_string_lossy(),
                schema.version.value,
                schema.declarations.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}: {error}", path.to_string_lossy());
            ExitCode::from(1)
        }
    }
}
