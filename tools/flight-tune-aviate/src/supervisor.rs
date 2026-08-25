//! Internal owner and launch-gate entry points.

pub(crate) mod config;
mod gate;
mod owner;
mod process_control;

pub(crate) use config::{BOOTSTRAP_SCHEMA_VERSION, SupervisorBootstrap};

use crate::AviateSupervisorError;

pub(crate) fn run_from_arguments() -> Result<(), AviateSupervisorError> {
    let mode = std::env::args()
        .nth(1)
        .ok_or_else(|| AviateSupervisorError::invalid_request("the supervisor mode is missing"))?;
    match mode.as_str() {
        "supervise" => owner::run(),
        "gate" => gate::run(),
        _ => Err(AviateSupervisorError::invalid_request(
            "the supervisor mode is invalid",
        )),
    }
}
