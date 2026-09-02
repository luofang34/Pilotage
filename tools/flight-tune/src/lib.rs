//! Simulator-only flight parameter search and qualification.
//!
//! Adaptive search reads training evidence only. One frozen candidate receives
//! one hidden promotion comparison; a separate final qualification seals the
//! campaign. The journal saves a prepared attempt before simulator mutation and
//! states one retry decision for every quarantine. SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as flight_tune;

mod adapter;
mod campaign;
mod campaign_config;
mod contract_api;
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
    SimulatorSessionAcquisition, SimulatorSessionReceipt, SimulatorVehicleAdapter,
    SimulatorVehicleFactory, TelemetrySample, TransitionBindingReceipt, VehicleBinding,
    VehicleBindingAcquisition, VehicleBindingReceipt, VehicleBindingRollback, conformance,
};
pub use campaign::{StopReason, Tuner, TuningSummary};
pub use campaign_config::{
    CAMPAIGN_CONFIG_SCHEMA_VERSION, CampaignAdapterDocuments, CampaignConfig,
};
pub use contract_api::*;
pub use error::{OpenRollbackOperation, OpenRollbackReport, TuneError};
pub use flight_quality::{
    CanonicalTelemetryKey, EVALUATOR_IMPLEMENTATION_SCHEMA_VERSION, FlightQualityGate,
    FlightQualityGateConfig, FlightQualityGateEvaluator, FlightQualityMetricConfig,
    FlightQualityMetricEvaluator, FlightQualityReport, FlightQualityScales, FlightQualityScenario,
    FlightQualityWeights, GATE_IMPLEMENTATION_ID, MANDATORY_CRASH_GATE_ID,
    METRIC_IMPLEMENTATION_ID, ReleasePlan, StepPlan, WindPlan,
};
pub use identity::{
    ArtifactIdentity, CandidateLineage, EvaluatorIdentities, RuntimeIdentities,
    scenario_engine_identity, scenario_runtime_identity,
};
pub use journal::{
    ATTEMPT_PROJECTION_SCHEMA_VERSION, AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION,
    AttemptProjection, AttemptProjectionOutcome, AttemptProjectionRecord, AttemptRetryOutcome,
    AttemptRole, AuthenticatedEvaluationProof, AuthenticatedJournalHead,
    AuthenticatedJournalRecord, CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION,
    CampaignEvidenceAuthority, CampaignPhase, FinalQualificationOutcome,
    JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION, Journal, JournalEntry, JournalEvent,
    JournalEvidenceSnapshot, OperationStatus, PROMOTION_CLOSURE_SCHEMA_VERSION, PromotionClosure,
    PromotionDecision, SessionIdentity, quarantine_reason_digest,
};
pub use model::{
    CampaignRunBound, Candidate, EXECUTION_RETRY_POLICY_SCHEMA_VERSION, ExecutionRetryPolicy,
    ExpectedPromotionPair, ExpectedPromotionRun, MissionReference, PROMOTION_POLICY_SCHEMA_VERSION,
    ParameterBounds, PhysicalTarget, PromotionCalculation, PromotionComparison,
    PromotionObjectiveResult, PromotionPairedStatistics, PromotionPolicy, PromotionRunKey,
    PromotionRunPlan, PromotionScenarioResults, PromotionSeedPolicy, PromotionSelection,
    QualificationPolicy, RESPONSE_TARGET_TABLE_SCHEMA_VERSION, ResponseTargetScope,
    ResponseTargetTable, ScenarioMotion, ScopedResponseTarget, SearchGroup, SearchGroupBinding,
    SearchGroupKind, SearchStage, TARGET_AUTHORITY_OBJECTIVE, TRAINING_SUITE_SCHEMA_VERSION,
    TargetAuthorityBand, TargetComparison, TrainingSuite, is_admissible, promotion_policy_digest,
};
pub use run_context::{RUN_EXECUTION_CONTEXT_SCHEMA_VERSION, RunExecutionContext};
pub use scenario_runtime::{
    CampaignMissionRuntime, ConditionAdmission, KinematicTruth, ReferenceStimulus, ScenarioFrame,
    ScenarioObservationReceipt, ScenarioRuntime, ScenarioRuntimeError, ScenarioStopContext,
    ScenarioStopReason, UncertaintyDeclaration, calibration_mission_document,
    mission_document_from_scenario, reference_observation_scenario, reference_stimulus_scenario,
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
