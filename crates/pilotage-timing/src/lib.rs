//! Time model for Pilotage: monotonic timestamps and simulation ticks.
//!
//! This crate is sans-IO: it never reads a system clock. Callers supply `now`
//! as an explicit parameter, per ADR-0002 and ADR-0009's time-domain split.

mod estimate;
mod latency;
mod staleness;
mod stamp;

pub use estimate::{ClockOffset, RttEstimator, estimated_age};
pub use latency::{BoundedLatencyLog, Stage, StageLatency};
pub use staleness::{Freshness, StalenessPolicy};
pub use stamp::{MonoTimestamp, SimTick};
