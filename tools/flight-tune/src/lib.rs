//! Simulator-only flight parameter search and qualification.
//!
//! Adaptive search can read training evidence only. One frozen candidate then
//! receives one hidden promotion comparison. A separate final qualification
//! seals the campaign. The journal saves a prepared attempt before simulator
//! mutation and quarantines interrupted work before cleanup.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as flight_tune;

mod adapter;
mod engine;
mod error;
mod flight_quality;
mod identity;
mod journal;
mod model;
mod run_context;
mod score;
mod strategy;
mod terminal;

pub use adapter::{
    AdapterError, CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION, CandidateReceipt,
    CandidateTransitionReceipt, CandidateTransitionReference, CandidateTransitionRequest,
    EvaluatorError, RunPreparationReceipt, RunTerminalAdapter, RunTerminalCapabilities,
    SampleEvent, ScenarioStartReceipt, SessionChallenge, SimulatorBackend, SimulatorCapability,
    SimulatorSessionReceipt, SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample,
    TransitionBindingReceipt, VehicleBinding, VehicleBindingReceipt,
};
pub use engine::{StopReason, Tuner, TuningSummary};
pub use error::TuneError;
pub use flight_quality::{
    CanonicalTelemetryKey, FlightQualityGate, FlightQualityGateConfig, FlightQualityGateEvaluator,
    FlightQualityMetricConfig, FlightQualityMetricEvaluator, FlightQualityReport,
    FlightQualityScales, FlightQualityScenario, FlightQualityWeights, ReleasePlan, StepPlan,
    WindPlan,
};
pub use identity::{ArtifactIdentity, CandidateLineage, RuntimeIdentities};
pub use journal::{
    AttemptRole, CampaignPhase, FinalQualificationOutcome, Journal, JournalEntry, JournalEvent,
    OperationStatus, PromotionDecision, SessionIdentity,
};
pub use model::{
    Candidate, ParameterBounds, PromotionPolicy, QualificationPolicy, ScenarioRef, SearchStage,
};
pub use pilotage_trial::Digest;
pub use run_context::{RUN_EXECUTION_CONTEXT_SCHEMA_VERSION, RunExecutionContext};
pub use score::{
    CandidateEvaluation, ConfidenceInterval, GateEvaluator, GateOutcome, HardGateFailure,
    MetricEvaluator, MetricValues, RunRecord, ScenarioSet, ScoreAggregate,
};
pub use strategy::{
    BoundedCoordinateSearch, Proposal, ProposalContext, ProposalError, ProposalStrategy,
    TrainingObservation, TrainingView,
};
pub use terminal::{
    MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES, RUN_BINDING_RECEIPT_SCHEMA_VERSION,
    RUN_TERMINAL_CLASS_SCHEMA_VERSION, RUN_TERMINAL_INTENT_SCHEMA_VERSION,
    RUN_TERMINAL_OPERATION_ORDER, RUN_TERMINAL_PLAN_SCHEMA_VERSION,
    RUN_TERMINAL_RECEIPT_SCHEMA_VERSION, RUN_TERMINAL_REPORT_SCHEMA_VERSION, RunBindingReceipt,
    RunTerminalClass, RunTerminalCompletion, RunTerminalDiagnostic, RunTerminalDisposition,
    RunTerminalIntent, RunTerminalOperation, RunTerminalOperationOutcome,
    RunTerminalOperationStatus, RunTerminalPlan, RunTerminalQuarantine, RunTerminalReceipt,
    RunTerminalRecoveryState, RunTerminalReport, RunTerminalRequirement, RunTerminalScope,
    RunTerminalSemanticOutcome, run_terminal_policy_digest,
};
