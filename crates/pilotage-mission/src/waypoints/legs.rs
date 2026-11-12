//! Altitude constraints, course references, turn directions, hold
//! racetracks, and leg path terminators for planned waypoints.

use navigate_contract::AltitudeConstraint;
use sha2::{Digest as _, Sha256};

use crate::engine::LegRole;

/// Altitude constraint at a planned waypoint, metres above the WGS84
/// ellipsoid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlannedAltitude {
    /// Cross at this altitude.
    At(f64),
    /// Cross at or above this altitude.
    AtOrAbove(f64),
    /// Cross at or below this altitude.
    AtOrBelow(f64),
    /// Cross between `lower_m` and `upper_m`.
    Window {
        /// Lower bound, metres.
        lower_m: f64,
        /// Upper bound, metres.
        upper_m: f64,
    },
    /// No crossing constraint; `reference_m` supplies the 3D fix height.
    Unconstrained {
        /// Reference height of the fix above the ellipsoid, metres.
        reference_m: f64,
    },
}

impl PlannedAltitude {
    /// The altitude the waypoint's 3D position stands at: the constraint's
    /// value, the midpoint of a window, or `reference_m` when unconstrained.
    #[must_use]
    pub fn reference_m(&self) -> f64 {
        match *self {
            Self::At(v)
            | Self::AtOrAbove(v)
            | Self::AtOrBelow(v)
            | Self::Unconstrained { reference_m: v } => v,
            Self::Window { lower_m, upper_m } => 0.5 * (lower_m + upper_m),
        }
    }

    /// Converts to Navigate's [`AltitudeConstraint`], or `None` when
    /// unconstrained.
    #[must_use]
    pub const fn constraint(&self) -> Option<AltitudeConstraint> {
        match *self {
            Self::At(v) => Some(AltitudeConstraint::At(v)),
            Self::AtOrAbove(v) => Some(AltitudeConstraint::AtOrAbove(v)),
            Self::AtOrBelow(v) => Some(AltitudeConstraint::AtOrBelow(v)),
            Self::Window { lower_m, upper_m } => {
                Some(AltitudeConstraint::Window { lower_m, upper_m })
            }
            Self::Unconstrained { .. } => None,
        }
    }

    pub(crate) fn hash(&self, hasher: &mut Sha256) {
        let (tag, values) = match *self {
            Self::At(v) => (1, [v, 0.0]),
            Self::AtOrAbove(v) => (2, [v, 0.0]),
            Self::AtOrBelow(v) => (3, [v, 0.0]),
            Self::Window { lower_m, upper_m } => (4, [lower_m, upper_m]),
            Self::Unconstrained { reference_m } => (5, [reference_m, 0.0]),
        };
        hasher.update([tag]);
        for value in values {
            hasher.update(value.to_le_bytes());
        }
    }
}

/// North reference from which a planned course is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedCourseReference {
    /// True north.
    True,
    /// Magnetic north (`P4`), converted to true north using WMM2025 at the
    /// mission's configured [`pilotage_world::MagneticEpoch`]. Without a
    /// configured epoch, activation refuses the magnetic course by name.
    Magnetic,
}

/// Mandatory turn direction onto a leg or inside a holding racetrack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaypointTurnDirection {
    /// Counterclockwise (left) turn.
    Left,
    /// Clockwise (right) turn.
    Right,
}

/// Inbound leg dimension of a planned holding pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlannedHoldLength {
    /// Inbound leg time in minutes (for example `1.0` or `1.5`).
    Minutes(f64),
    /// Inbound leg length in metres.
    DistanceM(f64),
}

/// Racetrack geometry of a planned hold leg (`HA`, `HF`, or `HM`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannedHold {
    /// Inbound course to the hold fix, radians in `[0, 2π)` from
    /// `reference` north.
    pub inbound_course_rad: f64,
    /// North reference of `inbound_course_rad`.
    pub reference: PlannedCourseReference,
    /// Turn direction of the holding racetrack (`L` or `R`).
    pub turn: WaypointTurnDirection,
    /// Inbound leg time or distance.
    pub length: PlannedHoldLength,
}

impl PlannedHold {
    fn hash(&self, hasher: &mut Sha256) {
        hasher.update(self.inbound_course_rad.to_le_bytes());
        hash_reference(self.reference, hasher);
        hash_turn_direction(Some(self.turn), hasher);
        match self.length {
            PlannedHoldLength::Minutes(minutes) => {
                hasher.update([1]);
                hasher.update(minutes.to_le_bytes());
            }
            PlannedHoldLength::DistanceM(meters) => {
                hasher.update([2]);
                hasher.update(meters.to_le_bytes());
            }
        }
    }
}

/// Path and termination type of the leg ending at a planned waypoint.
/// When absent on [`super::PlannedWaypoint`], Navigate resolves index `0`
/// to `InitialFix` and subsequent legs to `TrackToFix`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlannedPath {
    /// Initial fix (`IF`) starting a procedure or segment.
    InitialFix,
    /// Great-circle track from the preceding fix (`TF`).
    TrackToFix,
    /// Great-circle track from present position (`DF`).
    DirectToFix,
    /// Published course terminating at the fix (`CF`, `P4`).
    CourseToFix {
        /// Course in radians `[0, 2π)` clockwise from `reference` north.
        course_rad: f64,
        /// North reference of `course_rad`.
        reference: PlannedCourseReference,
    },
    /// Published course terminating when the waypoint's altitude constraint
    /// is achieved (`CA`, `P4`).
    CourseToAltitude {
        /// Course in radians `[0, 2π)` clockwise from `reference` north.
        course_rad: f64,
        /// North reference of `course_rad`.
        reference: PlannedCourseReference,
    },
    /// Hold-to-altitude (`HA`): fly the racetrack until the waypoint's
    /// altitude constraint is reached.
    HoldToAltitude(PlannedHold),
    /// Hold-to-fix (`HF`): enter the racetrack, complete one circuit to the
    /// hold fix, and sequence.
    HoldToFix(PlannedHold),
    /// Hold-to-manual termination (`HM`): fly circuits until operator/pilot
    /// exit is armed.
    HoldManual(PlannedHold),
}

impl PlannedPath {
    pub(crate) fn hash(path: Option<&Self>, hasher: &mut Sha256) {
        match path {
            None => hasher.update([0]),
            Some(Self::TrackToFix) => hasher.update([1]),
            Some(Self::DirectToFix) => hasher.update([2]),
            Some(Self::CourseToFix {
                course_rad,
                reference,
            }) => {
                hasher.update([3]);
                hasher.update(course_rad.to_le_bytes());
                hash_reference(*reference, hasher);
            }
            Some(Self::InitialFix) => hasher.update([4]),
            Some(Self::CourseToAltitude {
                course_rad,
                reference,
            }) => {
                hasher.update([5]);
                hasher.update(course_rad.to_le_bytes());
                hash_reference(*reference, hasher);
            }
            Some(Self::HoldToAltitude(hold)) => {
                hasher.update([6]);
                hold.hash(hasher);
            }
            Some(Self::HoldToFix(hold)) => {
                hasher.update([7]);
                hold.hash(hasher);
            }
            Some(Self::HoldManual(hold)) => {
                hasher.update([8]);
                hold.hash(hasher);
            }
        }
    }
}

fn hash_reference(reference: PlannedCourseReference, hasher: &mut Sha256) {
    hasher.update([match reference {
        PlannedCourseReference::True => 1,
        PlannedCourseReference::Magnetic => 2,
    }]);
}

pub(crate) fn hash_turn_direction(direction: Option<WaypointTurnDirection>, hasher: &mut Sha256) {
    hasher.update([match direction {
        None => 0,
        Some(WaypointTurnDirection::Left) => 1,
        Some(WaypointTurnDirection::Right) => 2,
    }]);
}

pub(crate) fn hash_role(role: Option<LegRole>, hasher: &mut Sha256) {
    hasher.update([match role {
        None => 0,
        Some(LegRole::Departure) => 1,
        Some(LegRole::Enroute) => 2,
        Some(LegRole::Arrival) => 3,
        Some(LegRole::Approach) => 4,
        Some(LegRole::MissedApproach) => 5,
    }]);
}

/// Hashes an optional `f64` with a presence tag.
pub(crate) fn hash_optional(value: Option<f64>, hasher: &mut Sha256) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}
