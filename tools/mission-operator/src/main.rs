//! `mission-operator`: a headless operator for a mission lane (ADR-0052).
//! It connects to one host as an ordinary client and sends the commands
//! every client sends, through the same mission desk as the App.
//!
//! ```text
//! mission-operator --host URL --cert HEX [--token-file PATH] [--name NAME] COMMAND
//!   propose OBJECTIVE.json
//!   approve [DIGEST] [--max-height M] [--expires-min N] [--refly N] [--no-revise] [--reserve F]
//!   start [DIGEST] | hold | resume | abort
//!   watch [--seconds N]
//!   run OBJECTIVE.json --approve [policy options] [--seconds N]
//!   agent
//!   survey-agent OBJECTIVE.json
//! ```
//!
//! `run` proposes, approves under the stated policy, starts and watches
//! until the lane ends; `--approve` states the operator's approval
//! explicitly. `agent` reads commands from standard input, one JSON object
//! per line such as `{"command": "hold"}`, for an automated agent.
//! `survey-agent` is such an agent, scripted: it proposes, waits for a
//! person to approve, starts, and resumes only the holds it may. Every
//! receipt and status goes to standard output as one JSON object per line.
//! The host decides what the client may do from the secret in
//! `--token-file`: only its approver secret lets a client approve.

mod cli;
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
