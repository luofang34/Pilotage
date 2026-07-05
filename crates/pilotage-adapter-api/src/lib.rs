//! Adapter traits and the capability model that engine-specific adapters
//! implement (ADR-0008).
//!
//! This crate is sans-IO: it defines the boundary traits only. Engine SDK
//! calls and I/O live in adapter implementations such as
//! `pilotage-adapter-reference`, per ADR-0002.

mod capability;
mod control;
mod step;
mod telemetry;
mod vehicle_adapter;

pub use capability::{AdapterCapabilities, ExecutionMode, ScopeDescriptor, VehicleDescriptor};
pub use control::{ApplyOutcome, Disposition, LinkLossPolicy, RejectReason};
pub use step::{StepBudget, StepOutcome};
pub use telemetry::{Pose2d, TelemetryBatch, TelemetrySample, VideoSource};
pub use vehicle_adapter::VehicleAdapter;
