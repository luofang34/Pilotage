//! Repository orchestration entry point (`cargo xtask ...`): launches a
//! full SITL session behind one command with event-based readiness and
//! ordered teardown, and wraps the simulation reset script.

use std::process::ExitCode;

mod affected;
mod backend;
mod cli;
mod error;
mod guards;
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
        Command::Handshake(out_dir) => session::run_handshake(&out_dir),
        Command::Guards => guards::run_guards(&session::repo_root()?),
        Command::Affected { base } => affected::run_affected(&base),
        Command::Sim(sim) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| error::XtaskError::Io {
                    context: "building the async runtime",
                    source,
                })?;
            let outcome = runtime.block_on(session::run_sim(&sim));
            // Dropping a runtime WAITS for blocking tasks that already
            // started. A stop requested while the session was waiting on the
            // simulator leaves one of those behind, so the drop would hold the
            // process for the rest of that task's timeout — after the stop was
            // acknowledged and after every child was killed. The operator gets
            // a dead terminal with nothing running and nothing on screen, and
            // ctrl-c cannot shorten it: tokio's signal handler is
            // process-global and outlives the runtime it was registered on.
            //
            // Everything the session owed has been torn down and its outcome
            // is in hand, so there is nothing left to wait for.
            runtime.shutdown_background();
            outcome
        }
    }
}
