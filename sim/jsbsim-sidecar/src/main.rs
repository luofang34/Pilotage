//! `pilotage-jsbsim-sidecar`: flies one JSBSim vehicle on the shared
//! world for a PX4 SITL connected through its MAVLink simulator link.

use std::process::ExitCode;

use pilotage_jsbsim_sidecar::args::{Args, usage};
use pilotage_jsbsim_sidecar::run::run_blocking;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv) {
        Ok(args) => args,
        Err(error) => {
            tracing::error!(%error, usage = %usage(), "bad arguments");
            return ExitCode::from(2);
        }
    };
    match run_blocking(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the sidecar stopped");
            ExitCode::FAILURE
        }
    }
}
