//! Diagnostic CLI for probing locally connected HID devices during native
//! input-port development. Not part of the sans-IO core (ADR-0002).
//!
//! See `cli` for the exact command grammar.

mod artifact_file;
mod capture_cmd;
mod characterize_cmd;
mod cli;
mod decode;
mod device;
mod error;
mod list_cmd;
mod output;
mod promote_cmd;
mod read_cmd;

use std::process::ExitCode;

use cli::Command;
use error::ProbeError;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init()
        .ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "hid-probe failed");
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
        Command::Capture {
            idle_seconds,
            movement_seconds,
            logical_axes,
            out,
        } => capture_cmd::run(idle_seconds, movement_seconds, &logical_axes, &out),
        Command::Characterize {
            contract,
            capture,
            profile,
            out,
        } => characterize_cmd::run(&contract, &capture, &profile, &out),
        Command::Promote {
            contract,
            capture,
            candidate,
            profile,
            out,
            confirmed_source_digest,
            confirmed_candidate_digest,
        } => promote_cmd::run(
            &contract,
            &capture,
            &candidate,
            &profile,
            &out,
            &confirmed_source_digest,
            &confirmed_candidate_digest,
        ),
    }
}
