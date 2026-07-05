//! Diagnostic CLI for probing locally connected HID devices during native
//! input-port development. Not part of the sans-IO core (ADR-0002).
//!
//! Subcommands: `list`, `read --seconds N`, `capture --seconds N --out PATH`.
//! See `cli` for the exact argument grammar.

mod capture_cmd;
mod cli;
mod decode;
mod device;
mod error;
mod list_cmd;
mod output;
mod read_cmd;

use std::process::ExitCode;

use cli::Command;
use error::ProbeError;
use output::print_line;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_line(&format!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// Parses `args` and dispatches to the matching subcommand. Kept separate
/// from `main` so subcommand errors are ordinary `Result` values rather than
/// a `process::exit` call (ADR-0015 bans the latter).
fn run(args: &[String]) -> Result<(), ProbeError> {
    match cli::parse_args(args)? {
        Command::List => list_cmd::run(),
        Command::Read { seconds } => read_cmd::run(seconds),
        Command::Capture { seconds, out } => capture_cmd::run(seconds, &out),
    }
}
