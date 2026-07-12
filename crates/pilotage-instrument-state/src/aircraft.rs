//! The raw input state a feeder writes.

use crate::quat::Quat;

/// Attitude estimate: orientation and body rotation rates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attitude {
    /// Body→NED rotation.
    pub quat: Quat,
    /// Body rates (p, q, r) in radians/second.
    pub rates_rps: [f32; 3],
}

/// Kinematic estimate in the local NED frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kinematics {
    /// Position (north, east, down) in meters from the local origin.
    pub pos_ned_m: [f32; 3],
    /// Velocity (north, east, down) in meters/second.
    pub vel_ned_mps: [f32; 3],
}

/// Air data. Every field is optional because vehicles without the sensor
/// must display `Missing`, not a substitute (ADR-0017).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AirData {
    /// Indicated airspeed in meters/second.
    pub ias_mps: Option<f32>,
    /// Altimeter setting in hectopascals.
    pub baro_setting_hpa: Option<f32>,
}

/// The selected lateral navigation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSource {
    /// No source selected; the HSI is a directional gyro.
    #[default]
    None,
    /// GPS/FMS course (magenta).
    Gps,
    /// NAV radio 1 (green).
    Nav1,
    /// NAV radio 2 (green).
    Nav2,
}

/// TO/FROM resolution of the selected course.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavFromTo {
    /// No valid course guidance; the deviation bar is removed.
    #[default]
    Off,
    /// Flying toward the station/waypoint.
    To,
    /// Flying away from the station/waypoint.
    From,
}

/// Lateral/vertical course guidance from the selected source.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NavData {
    /// Which source drives the deviation bar.
    pub source: NavSource,
    /// Selected course in radians.
    pub course_rad: f32,
    /// Lateral deviation in dots (full scale ±2).
    pub cdi_dots: f32,
    /// TO/FROM flag.
    pub fromto: NavFromTo,
    /// Vertical deviation in dots (full scale ±2.5), when available.
    pub vdev_dots: Option<f32>,
    /// Distance to the waypoint/station in nautical miles.
    pub dist_nm: Option<f32>,
}

/// Pilot selections and bugs. These are local UI state, not sensed data,
/// so they carry no freshness.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Selections {
    /// Heading bug in radians.
    pub heading_bug_rad: f32,
    /// Selected altitude in meters, when set.
    pub altitude_sel_m: Option<f32>,
}

/// Wind estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wind {
    /// Direction the wind blows *from*, radians clockwise from north.
    pub from_rad: f32,
    /// Speed in meters/second.
    pub speed_mps: f32,
}

/// Source-reported estimate quality (mirrors Aviate's `EstimateQuality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstimateQuality {
    /// Full confidence.
    #[default]
    Good,
    /// Reduced confidence; signals show `Degraded`.
    Degraded,
    /// The source says do not trust; signals show `Failed`.
    Unusable,
}

/// Which estimate groups the source declares valid (mirrors Aviate's
/// `StateValidFlags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidFlags {
    /// Attitude quaternion is valid.
    pub attitude: bool,
    /// Body rates are valid.
    pub rates: bool,
    /// NED position is valid.
    pub position: bool,
    /// NED velocity is valid.
    pub velocity: bool,
}

impl Default for ValidFlags {
    fn default() -> Self {
        Self {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
        }
    }
}

/// One estimate group with the age a feeder stamped it with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stamped<T> {
    /// The data, absent until first received.
    pub data: Option<T>,
    /// Milliseconds since last update; `None` when never received.
    pub age_ms: Option<f32>,
}

impl<T> Default for Stamped<T> {
    fn default() -> Self {
        Self {
            data: None,
            age_ms: None,
        }
    }
}

/// Whether independently acquired groups form one coherent display snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapshotCoherence {
    /// Too few stamped groups are present to establish coherence.
    #[default]
    Insufficient,
    /// Required groups share a source epoch/clock and meet the skew budget.
    Coherent,
    /// Required groups exceed the configured acquisition-time skew budget.
    ExcessiveSkew,
}

/// Metadata assigned by the ingress gate to one immutable state generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapshotMeta {
    /// Wrapping generation advanced only when a source group advances.
    pub generation: u32,
    /// Coherence result for the independently stamped input groups.
    pub coherence: SnapshotCoherence,
}

/// The unified input state every instrument reads (ADR-0017).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AircraftState {
    /// Attitude group.
    pub attitude: Stamped<Attitude>,
    /// Kinematics group.
    pub kinematics: Stamped<Kinematics>,
    /// Air-data group.
    pub air: Stamped<AirData>,
    /// Navigation guidance group.
    pub nav: Stamped<NavData>,
    /// Wind estimate group.
    pub wind: Stamped<Wind>,
    /// Pilot selections.
    pub selections: Selections,
    /// Source quality.
    pub quality: EstimateQuality,
    /// Source validity flags.
    pub valid: ValidFlags,
    /// Ingress generation and group-coherence result.
    pub snapshot: SnapshotMeta,
}
