use serde::{Deserialize, Serialize};

/// One scalar value at one trial time.
///
/// The caller supplies seconds and the documented unit for the metric. The
/// caller must project vector data onto one axis before it builds this value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimedValue {
    /// Trial time, in seconds.
    pub time_s: f64,
    /// The scalar value.
    pub value: f64,
}

/// One position and velocity point for a release metric.
///
/// Position is in meters. Velocity is in meters per second. Both values use
/// the same selected axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPoint {
    /// Trial time, in seconds.
    pub time_s: f64,
    /// Position on the selected axis, in meters.
    pub position_m: f64,
    /// Velocity on the selected axis, in meters per second.
    pub velocity_mps: f64,
}

/// One actuator-demand point for a control metric.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPoint {
    /// Trial time, in seconds.
    pub time_s: f64,
    /// Normalized control demand.
    pub effort: f64,
    /// Whether an actuator limit is active.
    pub saturated: bool,
}
