//! Deterministic offline flight-quality metrics and hard trial gates.
//!
//! This crate reads scalar projections from a saved trial. It does not own the
//! trial schema. A caller selects one phase and one axis before it creates a
//! metric input series.
//!
//! The metric rules are fixed. The scorer uses linear interpolation between
//! samples. It uses time weights for distributions. It does not filter input
//! data. It calculates jerk from adjacent acceleration samples.
//!
//! Hard gates are separate from continuous metrics. A hard gate failure cannot
//! become a passing result through a continuous score.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

/// The identity of the fixed numerical rules in this crate.
pub const FLIGHT_QUALITY_SCORER_VERSION: u16 = 1;

mod control;
mod error;
mod gate;
mod release;
mod response;
mod sample;
mod series;
mod signal;
mod vocabulary;

pub use control::{ControlMetrics, measure_control};
pub use error::MetricError;
pub use gate::{GateContext, HardGate, HardGateOutcome, HardGateReport};
pub use release::{
    HOLD_ZERO_HYSTERESIS_M, HoldMetrics, ReleaseMetrics, STOP_DWELL_S, STOP_SPEED_MPS,
    measure_hold, measure_release,
};
pub use response::{
    DELAY_FRACTION, RISE_HIGH_FRACTION, RISE_LOW_FRACTION, ResponseMetrics, SETTLING_FRACTION,
    STEADY_STATE_WINDOW_S, StepSpec, measure_step_response,
};
pub use sample::{ControlPoint, MotionPoint, TimedValue};
pub use signal::{JerkMetrics, SignalStats, measure_jerk, measure_signal};
pub use vocabulary::{
    CONTROL_METRICS, HOLD_METRICS, JERK_METRICS, RELEASE_METRICS, RESPONSE_METRICS, SIGNAL_METRICS,
    is_producible, producible_metrics,
};

#[cfg(test)]
mod test_trace;
