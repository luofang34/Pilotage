//! Aviate process supervision and direct control for simulator tuning.
//!
//! The supervisor keeps one target behind a launch gate until it stores and
//! reads back the exact process identity. A parent-lifetime pipe requests
//! cleanup after parent failure. Recovery does not send process signals. The
//! launch gate and its owner contain the exact private process group.
//!
//! [`direct_transport`] carries the simulator-only exact direct setpoints
//! that qualify a direct controller. It is not reachable from the normal
//! application control interface.
//!
//! This crate supports only Linux and macOS.
//!
//! Isolate runtime, storage, and artifact roots from non-cooperating code that
//! runs as the same user.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod action_port;
mod artifact;
pub mod direct_transport;
mod document;
mod error;
mod inspection;
mod lease_store;
mod process;
mod protocol;
mod runtime_files;
mod supervisor;

pub use action_port::{
    AviateActionDriver, AviateActionPortError, AviateVehicleActionPort, aviate_action_port_identity,
};
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
