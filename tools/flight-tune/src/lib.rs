//! Simulator-only flight parameter search and qualification.
//!
//! Adaptive search can read training evidence only. One frozen candidate then
//! receives one hidden promotion comparison. A separate final qualification
//! seals the campaign. The journal saves a prepared attempt before simulator
//! mutation and quarantines interrupted work before cleanup.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod adapter;
mod engine;
mod error;
mod flight_quality;
mod identity;
mod journal;
mod model;
mod score;
mod strategy;

pub use adapter::{
    AdapterError, CandidateReceipt, EvaluatorError, SampleEvent, ScenarioStartReceipt,
    SessionChallenge, SimulatorBackend, SimulatorCapability, SimulatorSessionReceipt,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample, VehicleBinding,
    VehicleBindingReceipt,
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
pub use score::{
    CandidateEvaluation, ConfidenceInterval, GateEvaluator, GateOutcome, HardGateFailure,
    MetricEvaluator, MetricValues, RunRecord, ScenarioSet, ScoreAggregate,
};
pub use strategy::{
    BoundedCoordinateSearch, Proposal, ProposalContext, ProposalError, ProposalStrategy,
    TrainingObservation, TrainingView,
};
