use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = tidy::cli::Cli::parse();
    match tidy::app::run(cli) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("tidy: {err}");
            ExitCode::from(1)
        }
    }
}
