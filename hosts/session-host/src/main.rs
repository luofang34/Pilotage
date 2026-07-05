//! Binary entry point for `pilotage-session-host`; see the crate root
//! (`lib.rs`) for the module overview.

use std::process::ExitCode;

use pilotage_session_host::error::HostError;
use pilotage_session_host::{cli, runtime};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "session host exited with an error");
            ExitCode::FAILURE
        }
    }
}

/// Parses arguments, builds the tokio runtime, and runs the host until
/// `ctrl_c`.
///
/// # Errors
///
/// Returns [`HostError`] if argument parsing or startup fails.
fn run(args: &[String]) -> Result<(), HostError> {
    let cli_args = cli::parse_args(args).map_err(HostError::Cli)?;
    let runtime = tokio::runtime::Runtime::new().map_err(HostError::Runtime)?;
    runtime.block_on(async move {
        let host = runtime::start(cli_args.port)?;
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for ctrl_c");
        }
        host.shutdown().await;
        Ok(())
    })
}
