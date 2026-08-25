//! Crash-contained Aviate process supervision for simulator tuning.
//!
//! The supervisor keeps one target behind a launch gate until it stores and
//! reads back the exact process identity. A parent-lifetime pipe requests
//! cleanup after parent failure. Recovery does not send process signals. The
//! launch gate and its owner contain the exact private process group.
//!
//! This crate supports only Linux and macOS.
//!
//! Isolate runtime, storage, and artifact roots from non-cooperating code that
//! runs as the same user.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod artifact;
mod document;
mod error;
mod inspection;
mod lease_store;
mod process;
mod protocol;
mod runtime_files;
mod supervisor;

pub use document::{
    ProcessIdentity, ProcessStartIdentity, TargetAttestation, TargetProcessContract,
};
pub use error::AviateSupervisorError;
pub use process::{
    ManagedAviateProcess, PreparedAviateProcess, RECOVERY_REQUEST_SCHEMA_VERSION, RecoveryOutcome,
    RecoveryRequest, SUPERVISION_ATTESTATION_SCHEMA_VERSION, SupervisedProcessRequest,
    SupervisionAttestation, recover_supervised_process_blocking,
};

/// Runs the process-supervisor helper protocol.
///
/// The workspace helper binary is the only intended caller.
#[doc(hidden)]
pub fn supervisor_main_blocking() -> Result<(), AviateSupervisorError> {
    supervisor::run_from_arguments()
}
