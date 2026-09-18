//! `intent-pilot`: the headless port of the agent client module.
//!
//! A model outside Pilotage reads each operator message and replies with a
//! directive. The agent core checks the directive and flies it through the
//! typed control path, as one more client beside the keyboard and the gamepad.
//! In a scenario run a verifier judges the flight from simulator truth, and
//! the exit status carries its verdict. In a live run a person types the
//! operator messages.

mod cli;
mod error;
mod link;
mod pilot;
mod record;
mod script;
mod session;
mod telemetry;

use std::io::Write;
use std::process::ExitCode;

use pilotage_agent::Verdict;

use crate::pilot::Outcome;

const EXIT_FAIL: u8 = 1;
const EXIT_INCONCLUSIVE: u8 = 2;
const EXIT_ERROR: u8 = 3;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Invocation::Fly(options)) => options,
        Ok(cli::Invocation::Help) => {
            emit(cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            tracing::error!(%error, "cannot start");
            emit(cli::USAGE);
            return ExitCode::from(EXIT_ERROR);
        }
    };
    // The runtime is built here and not by an attribute macro. The macro's
    // expansion carries lint attributes that the production lint gate forbids.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(source) => {
            let error = error::PilotError::Runtime(source);
            tracing::error!(%error, "cannot start");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    match runtime.block_on(pilot::fly(&options)) {
        Ok(Outcome::Live) => ExitCode::SUCCESS,
        Ok(Outcome::Verdict(report)) => {
            // The report line is the program's output, not a diagnostic.
            match serde_json::to_string(&report) {
                Ok(line) => emit(&line),
                Err(error) => tracing::error!(%error, "cannot encode the report"),
            }
            match report.verdict {
                Verdict::Pass => ExitCode::SUCCESS,
                Verdict::Fail => ExitCode::from(EXIT_FAIL),
                Verdict::Inconclusive => ExitCode::from(EXIT_INCONCLUSIVE),
            }
        }
        Err(error) => {
            let mut chain = String::new();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                chain.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            tracing::error!("the run stopped with no verdict: {error}{chain}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Writes one line of program output. A closed stdout is not an error
/// the pilot can act on.
fn emit(line: &str) {
    writeln!(std::io::stdout(), "{line}").ok();
}
