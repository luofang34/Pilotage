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

pub mod change;
pub mod config;
pub mod declared_start;
pub mod engine;
pub mod error;
pub mod fixture;
pub mod holding_pattern;
pub mod lane;
pub mod ownship;
pub mod policy;
pub mod provenance;
pub mod survey;
pub mod waypoints;

mod body;
mod body_frame;
mod route;

pub use body_frame::{
    project_cartesian_velocity_body, project_cartesian_velocity_ned, project_entry_bank_ned,
    project_guidance_setpoint, project_ned_velocity, project_thrust_vector_ned,
};
pub use config::{
    BeyondLimits, MissionConfig, MissionLimits, OwnshipSource, TerminalHoldingPattern,
};
pub use declared_start::{
    DeclaredHeading, DeclaredHeight, DeclaredPosition, DeclaredSpeed, DeclaredStart,
    DeclaredStartError, SpeedReference,
};
pub use engine::{
    APPROACH_RNP_NM, CifpProcedureKind, ENROUTE_RNP_NM, FlightPhase, HoldEntry, HoldStage,
    HoldStatus, LegRole, M_PER_NM, PositionIntegrity, PositionKind, PositionSource,
    ProcedureFixRole, SteeringIntegrity, SteeringSource, TERMINAL_RNP_NM,
};
pub use engine::{
    DeadReckoning, LaunchDetection, MissionAction, MissionCounters, MissionEngine, MissionEvent,
    MissionOutput, MissionState, NavGuidance, NavQuality, StartWithheldReason,
    VEHICLE_STATUS_MAX_AGE_NS, VehicleStatus,
};
pub use engine::{
    DisplayFix, DisplayWaypoint, LegDisplay, NavigationDisplay, ToFrom, WaypointProgress,
};
pub use engine::{LateralMode, LateralStatus, VerticalMode, VerticalStatus};
pub use engine::{OnboardSolution, SourceIntegrity, SourceIntegrityState};
pub use error::MissionBuildError;
pub use holding_pattern::{
    HoldingEntry, HoldingLeg, HoldingPattern, HoldingPatternStatus, HoldingTurn,
};
pub use lane::{
    Approvable, ApprovalTarget, FrameAssessment, FrameTicket, HOST_ISSUER, IntentContext, Lane,
    LaneConfig, LaneEffect, LaneReply, OperationAnswer, PlannedOperation, Reconciled, Requester,
    RunExecution, RunOrder, RunPlan, RunProgress, StatusQuery, Steering, SurveyDefaults,
    VerdictRefused, admit_approval, approval_target, operation_id, valid_approval,
};
pub use ownship::{OwnshipAltitude, OwnshipSample, TruthRole};
pub use pilotage_mission_core::ExecutionTarget;
pub use pilotage_mission_core::executor;
pub use provenance::{MissionPlanRecord, SnapshotProvenance, decode_snapshot};
pub use survey::{SurveyError, SurveyPlan, SurveyPlanRecord, SurveyRequest, plan_survey};
pub use waypoints::{
    DEFAULT_ARRIVAL_RADIUS_M, DEFAULT_ROUTE_BANK_DEG, MIN_CAPTURE_RADIUS_M, PlannedAltitude,
    PlannedCourseReference, PlannedHold, PlannedHoldLength, PlannedPath, PlannedWaypoint,
    WaypointPlan, WaypointPlanRecord, WaypointTurn, WaypointTurnDirection, capture_radius_for,
    indicated_airspeed_for, true_airspeed_for, turn_radius_for,
};
