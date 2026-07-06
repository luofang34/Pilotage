//! Native WebTransport client for the loopback session (increment 1):
//! connects to a session host, leases `vehicle.motion`, sends synthetic or
//! `--hid`-sourced control frames, and measures the loopback control loop.
//!
//! Not part of the sans-IO core (ADR-0002): this binary owns the
//! WebTransport connection, device I/O, and wall-clock timing.

mod cli;
mod control_source;
mod drive;
mod error;
mod hid_decode;
mod metrics;
mod output;
mod pipeline;
mod receiver;
mod run;
mod save_frames;
mod sender;
mod summary;
mod synthetic;
mod telemetry;
mod transport;
mod video;
mod wire_session;

use std::process::ExitCode;

use error::ProbeError;
use output::print_line;

fn main() -> ExitCode {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_blocking(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_line(&format!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// Parses arguments and drives the async probe run to completion on a
/// dedicated multi-thread runtime, blocking the calling thread.
///
/// Named `_blocking` (ADR-0015) since it synchronously waits out the whole
/// run rather than returning a future.
fn run_blocking(args: &[String]) -> Result<(), ProbeError> {
    let parsed = cli::parse_args(args)?;
    let runtime = tokio::runtime::Runtime::new().map_err(|source| ProbeError::Connect {
        message: format!("failed to start async runtime: {source}"),
    })?;
    runtime.block_on(run::run(&parsed))
}
