//! Repository orchestration entry point (`cargo xtask ...`): launches a
//! full SITL session behind one command with event-based readiness and
//! ordered teardown, and wraps the simulation reset script.

use std::process::ExitCode;

mod backend;
mod cli;
mod error;
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
        Err(error) => {
            print_line(&format!("error: {error}"));
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
        Command::Reset => session::run_reset("aviate"),
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
