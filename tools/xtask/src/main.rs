//! Repository orchestration entry point (`cargo xtask ...`): launches a
//! full SITL session behind one command with event-based readiness and
//! ordered teardown, and wraps the simulation reset script.

use std::process::ExitCode;

mod backend;
mod cli;
mod error;
mod log_archive;
mod output;
mod process;
mod readiness;
mod session;

use cli::Command;
use output::print_line;

fn main() -> ExitCode {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        // A ctrl-c before the session was ready is a requested stop:
        // everything started has been torn down, nothing failed.
        Err(error::XtaskError::Cancelled) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "xtask failed");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), error::XtaskError> {
    match cli::parse_args(args)? {
        Command::Help => {
            print_line(cli::USAGE);
            Ok(())
        }
        Command::Reset(fc) => session::run_reset(&fc),
        Command::Sim(sim) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| error::XtaskError::Io {
                    context: "building the async runtime",
                    source,
                })?;
            runtime.block_on(session::run_sim(&sim))
        }
    }
}
