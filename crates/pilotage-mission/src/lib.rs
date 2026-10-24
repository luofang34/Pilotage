//! Operational mission handler: a packed navdata snapshot plus a route
//! string becomes a validated flight plan and mission document. The shared
//! mission core sequences the document. Navigate interprets active flight
//! directives as typed velocity intents and discrete actions.
//!
//! The handler does not read a clock or perform input or output. `now` is a
//! caller-supplied [`navigate_contract::MonotonicNanos`] on the single
//! clock domain fixed in [`MissionConfig`]; telemetry arrives as
//! host-converted [`OwnshipSample`]s; outputs are
//! [`pilotage_protocol::ControlIntent`] and
//! [`pilotage_protocol::ControlAction`] values. Framing, sessions,
//! leases, and authority stay with the host task.

pub mod config;
pub mod engine;
pub mod error;
pub mod fixture;
pub mod lane;
pub mod ownship;
pub mod policy;
pub mod provenance;
pub mod survey;

mod body_frame;

pub use config::{MissionConfig, MissionLimits, OwnshipSource};
pub use engine::{
    MissionAction, MissionCounters, MissionEngine, MissionEvent, MissionOutput, MissionState,
    NavGuidance, NavQuality,
};
pub use error::MissionBuildError;
pub use lane::{
    Approvable, ApprovalTarget, FrameAssessment, FrameTicket, HOST_ISSUER, IntentContext, Lane,
    LaneConfig, LaneEffect, LaneReply, Requester, RunExecution, RunOrder, RunPlan, RunProgress,
    Steering, SurveyDefaults, VerdictRefused, approval_target, proposal_digest,
};
pub use ownship::{OwnshipAltitude, OwnshipSample, TruthRole};
pub use pilotage_mission_core::ExecutionTarget;
pub use provenance::{MissionPlanRecord, SnapshotProvenance, decode_snapshot};
pub use survey::{SurveyError, SurveyPlan, SurveyPlanRecord, SurveyRequest, plan_survey};
