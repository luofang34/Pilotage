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
pub mod condition;
pub mod direct_transport;
mod document;
mod error;
mod inspection;
mod lease_store;
mod process;
mod protocol;
pub mod runtime;
mod runtime_files;
mod supervisor;
mod transition_authorization;
mod vehicle;

pub use action_port::{
    AviateActionDriver, AviateActionPortError, AviateVehicleAction, AviateVehicleActionPort,
    AviateVehicleDirective, aviate_action_port_identity,
};
pub use condition::{
    AviateConditionError, ConditionLaunch, ConditionTracePath, TUNING_TRACE_SCHEMA_VERSION,
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
pub use runtime::identity::{
    AviateRuntimeIdentity, RUNTIME_IMPLEMENTATION_ID, RUNTIME_IMPLEMENTATION_SCHEMA_VERSION,
    RuntimeIdentityInputs, RuntimeImplementationDocument, RuntimeSourceEntry,
};
pub use runtime::{AviateRuntimeError, AviateScenarioDriver};
pub use transition_authorization::{
    ADJACENCY_POLICY_SCHEMA_VERSION, AdjacencyPolicy, ParameterStepLimit, TRANSITION_VALIDATOR_ID,
    TransitionValidator, validator_identity,
};
pub use vehicle::{
    AviateFeelController, AviateVehicleAdapter, AviateVehicleFactory, CandidateFeelMapping,
    VEHICLE_ID, bind_run_intent, require_run_intent, vehicle_identity,
};

/// Runs the process-supervisor helper protocol.
///
/// The workspace helper binary is the only intended caller.
#[doc(hidden)]
pub fn supervisor_main_blocking() -> Result<(), AviateSupervisorError> {
    supervisor::run_from_arguments()
}
