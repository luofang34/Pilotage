//! The raw input state a feeder writes.

use crate::altitude::{AltitudeClass, AltitudeDeclaration, GeoidModelId, OriginId};
use pilotage_frames::Quat;

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
    /// The wire carried a source this build does not know. Guidance from
    /// an unidentifiable source must not display; the nav group fails
    /// rather than quietly pretending no source is selected.
    Unknown,
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
    /// The wire carried a resolution this build does not know; the nav
    /// group fails rather than defaulting to a benign flag state.
    Unknown,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selections {
    /// Heading bug in radians.
    pub heading_bug_rad: f32,
    /// Selected altitude in meters, when set.
    pub altitude_sel_m: Option<f32>,
    /// Reference class the selected altitude is expressed in. The bug
    /// and selection readout render only against a compatible displayed
    /// reference — numeric equality across references means nothing,
    /// and class equality alone is not identity: the class-specific
    /// identity below must match too.
    pub altitude_sel_class: AltitudeClass,
    /// Origin identity of a local-relative selection. A selection made
    /// against origin A is not a selection against origin B.
    pub altitude_sel_origin: OriginId,
    /// Geoid-model identity of a geometric-MSL selection; undeclared is
    /// an incomplete identity and never compatible.
    pub altitude_sel_model: GeoidModelId,
    /// Pilot-selected altimeter setting in hectopascals. Selection is
    /// UI state; the sensed/applied setting lives in [`AirData`], and a
    /// disagreement between the two is flagged, never averaged.
    pub baro_sel_hpa: Option<f32>,
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
///
/// Trust must be declared, never assumed: the default is [`Self::Unknown`],
/// and a wire value outside the known set decodes to `Unknown` rather than
/// to a benign level. Unknown quality resolves `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstimateQuality {
    /// Full confidence.
    Good,
    /// Reduced confidence; signals show `Degraded`.
    Degraded,
    /// The source says do not trust; signals show `Failed`.
    Unusable,
    /// No quality was declared, or the declared value is not one this
    /// build knows; signals show `Failed`.
    #[default]
    Unknown,
}

/// Which estimate groups the source declares valid (mirrors Aviate's
/// `StateValidFlags`).
///
/// The default declares nothing valid: a feeder that never sets the flags
/// gets `Failed` groups, not silently trusted ones. Flags apply only to
/// groups that have data — a group never received stays `Missing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// The wire carried a coherence value this build does not know; the
    /// pairing cannot be trusted, so stamped groups degrade.
    Unknown,
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
    /// Datum declaration for the primary altitude (ALT-01).
    pub altitude: AltitudeDeclaration,
}

impl Default for Selections {
    fn default() -> Self {
        Self {
            heading_bug_rad: 0.0,
            altitude_sel_m: None,
            altitude_sel_class: AltitudeClass::LocalRelative,
            altitude_sel_origin: OriginId(0),
            altitude_sel_model: GeoidModelId::UNDECLARED,
            baro_sel_hpa: None,
        }
    }
}
