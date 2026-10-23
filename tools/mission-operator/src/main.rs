//! `mission-operator`: a headless operator for a mission lane (ADR-0052).
//! It connects to one host as an ordinary client and sends the operator's
//! intents: the same ones a client's mission panel sends.
//!
//! ```text
//! mission-operator --host URL --cert HEX propose OBJECTIVE.json
//! mission-operator --host URL --cert HEX approve DIGEST [--max-height M] [--expires-min N] [--refly N] [--no-revise]
//! mission-operator --host URL --cert HEX start DIGEST | hold | resume | abort
//! mission-operator --host URL --cert HEX watch [--seconds N]
//! mission-operator --host URL --cert HEX run OBJECTIVE.json --approve [envelope options] [--seconds N]
//! ```
//!
//! `run` proposes, approves under the stated envelope, starts and watches
//! until the lane ends; `--approve` states the operator's approval
//! explicitly. Every receipt and status goes to standard output as one
//! JSON object per line.

mod cli;
mod objective;
mod session;

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_blocking(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            writeln!(std::io::stderr().lock(), "error: {error}").ok();
            ExitCode::FAILURE
        }
    }
}

/// Parses the arguments and runs the command on its own runtime. Returns
/// whether the lane ended well, for `run` and `watch`.
fn run_blocking(args: &[String]) -> Result<bool, cli::OperatorError> {
    let command = cli::parse(args)?;
    let runtime = tokio::runtime::Runtime::new().map_err(cli::OperatorError::Runtime)?;
    runtime.block_on(session::execute(command))
}
