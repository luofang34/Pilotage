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
mod campaign;
mod campaign_config;
mod error;
mod flight_quality;
mod identity;
mod journal;
mod model;
mod run_context;
mod scenario_runtime;
mod score;
mod strategy;
mod terminal;

pub use adapter::{
    AdapterError, CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION, CampaignBackend, CandidateReceipt,
    CandidateTransitionReceipt, CandidateTransitionReference, CandidateTransitionRequest,
    EvaluatorError, RunPreparationReceipt, RunTerminalAdapter, RunTerminalCapabilities,
    SampleEvent, ScenarioStartReceipt, SessionChallenge, SimulatorCapability,
    SimulatorSessionReceipt, SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample,
    TransitionBindingReceipt, VehicleBinding, VehicleBindingReceipt,
};
pub use campaign::{StopReason, Tuner, TuningSummary};
pub use campaign_config::{
    CAMPAIGN_CONFIG_SCHEMA_VERSION, CampaignAdapterDocuments, CampaignConfig,
};
pub use error::TuneError;
pub use flight_quality::{
    CanonicalTelemetryKey, FlightQualityGate, FlightQualityGateConfig, FlightQualityGateEvaluator,
    FlightQualityMetricConfig, FlightQualityMetricEvaluator, FlightQualityReport,
    FlightQualityScales, FlightQualityScenario, FlightQualityWeights, ReleasePlan, StepPlan,
    WindPlan,
};
pub use identity::{
    ArtifactIdentity, CandidateLineage, RuntimeIdentities, scenario_engine_identity,
    scenario_runtime_identity,
};
pub use journal::{
    AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION, AttemptRole, AuthenticatedEvaluationProof,
    AuthenticatedJournalHead, AuthenticatedJournalRecord,
    CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION, CampaignEvidenceAuthority, CampaignPhase,
    FinalQualificationOutcome, JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION, Journal, JournalEntry,
    JournalEvent, JournalEvidenceSnapshot, OperationStatus, PROMOTION_CLOSURE_SCHEMA_VERSION,
    PromotionClosure, PromotionDecision, SessionIdentity,
};
pub use model::{
    Candidate, ExpectedPromotionPair, ExpectedPromotionRun, MissionReference,
    PROMOTION_POLICY_SCHEMA_VERSION, ParameterBounds, PromotionCalculation, PromotionComparison,
    PromotionObjectiveResult, PromotionPairedStatistics, PromotionPolicy, PromotionRunKey,
    PromotionRunPlan, PromotionSeedPolicy, PromotionSelection, QualificationPolicy, SearchStage,
    promotion_policy_digest,
};
pub use pilotage_mission_core::{
    ArtifactIdentity as MissionArtifactIdentity, ControlChannel, DirectiveContext, FlightAction,
    MISSION_SCHEMA_VERSION, MissionCapability, MissionDirective, MissionDocument, ObservedSignal,
    ReceiptResult, StartState, TrialAction, VehicleLifecycleState, Waveform,
};
pub use pilotage_trial::{Digest, Scenario as TrialScenario};
pub use run_context::{RUN_EXECUTION_CONTEXT_SCHEMA_VERSION, RunExecutionContext};
pub use scenario_runtime::{
    CampaignMissionRuntime, KinematicTruth, ScenarioFrame, ScenarioObservationReceipt,
    ScenarioRuntime, ScenarioRuntimeError, ScenarioStopContext, ScenarioStopReason,
    calibration_mission_document, mission_document_from_scenario, reference_observation_scenario,
};
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
    RunTerminalBindingStatus, RunTerminalClass, RunTerminalCompletion, RunTerminalDiagnostic,
    RunTerminalDisposition, RunTerminalIntent, RunTerminalOperation, RunTerminalOperationOutcome,
    RunTerminalOperationStatus, RunTerminalPlan, RunTerminalQuarantine, RunTerminalReceipt,
    RunTerminalRecoveryState, RunTerminalReport, RunTerminalRequirement, RunTerminalScope,
    RunTerminalSemanticOutcome, run_terminal_policy_digest,
};
