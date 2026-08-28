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
pub mod ownship;
pub mod policy;
pub mod provenance;

mod body_frame;

pub use config::{MissionConfig, MissionLimits};
pub use engine::{
    MissionAction, MissionCounters, MissionEngine, MissionEvent, MissionOutput, MissionState,
    NavGuidance, NavQuality,
};
pub use error::MissionBuildError;
pub use ownship::{OwnshipSample, TruthRole};
pub use provenance::{MissionPlanRecord, SnapshotProvenance, decode_snapshot};
