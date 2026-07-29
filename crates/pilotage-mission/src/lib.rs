//! Sans-IO mission engine: a packed navdata snapshot plus a route
//! string becomes a validated flight plan, and truth-role ownship
//! samples become typed velocity intents and discrete actions toward
//! the Pilotage control boundary.
//!
//! Nothing here reads a clock or performs I/O. `now` is always a
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
