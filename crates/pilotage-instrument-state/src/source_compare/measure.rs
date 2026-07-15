//! Function-specific comparison metrics: circular heading, q/-q-invariant
//! attitude, datum-qualified altitude, and explicit scalar/vector units.
//!
//! Each metric decides its own notion of "same datum" and "how far apart".
//! None converts across datums or units silently — a cross-datum pair is
//! not comparable, never coerced into agreement or disagreement. A
//! difference between same-unit samples is expressed in the metric's
//! canonical unit (radians, meters, meters/second), so a policy threshold
//! has exactly one meaning regardless of the unit a source reports in.

use libm::{acosf, sqrtf};

use pilotage_frames::Quat;

use crate::altitude::{AltitudeClass, GeoidModelId, OriginId};
use crate::heading::{HeadingReference, shortest_angle_rad};
use crate::source_compare::Comparable;
use crate::validate::validate_quat;

/// Identity of the reference frame an attitude is expressed in. Zero is
/// "undeclared": an attitude without a declared frame is ill-formed rather
/// than assumed to share any particular frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTag(pub u16);

impl FrameTag {
    /// No frame declared.
    pub const UNDECLARED: Self = Self(0);
}

/// An attitude sample compared by the q/-q-invariant geodesic angle between
/// rotations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeMeasure {
    /// The orientation as a unit quaternion.
    pub quat: Quat,
    /// The reference frame the rotation is expressed against.
    pub frame: FrameTag,
}

impl Comparable for AttitudeMeasure {
    fn well_formed(&self) -> bool {
        self.frame != FrameTag::UNDECLARED && validate_quat(self.quat).is_ok()
    }

    fn datum_compatible(&self, other: &Self) -> bool {
        self.frame != FrameTag::UNDECLARED && self.frame == other.frame
    }

    /// The geodesic angle `2·acos(|⟨q₁,q₂⟩|)` on SO(3). Reading only the
    /// absolute inner product makes `q` and `-q` produce a bit-identical
    /// result, mirroring the quadratic-form discipline used for horizon
    /// geometry.
    fn difference(&self, other: &Self) -> f32 {
        match (validate_quat(self.quat), validate_quat(other.quat)) {
            (Ok(a), Ok(b)) => {
                let dot = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
                2.0 * acosf(dot.abs().clamp(0.0, 1.0))
            }
            // Unreachable for a well-formed pair; a huge angle never agrees.
            _ => f32::INFINITY,
        }
    }
}

/// A heading sample compared by the shortest circular angle, so 359° and 1°
/// differ by 2°.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadingMeasure {
    /// Heading in radians clockwise from the declared north.
    pub heading_rad: f32,
    /// Which north the heading is measured from.
    pub reference: HeadingReference,
}

fn is_true_north(reference: HeadingReference) -> bool {
    matches!(
        reference,
        HeadingReference::True | HeadingReference::SimLocalTrue
    )
}

impl Comparable for HeadingMeasure {
    fn well_formed(&self) -> bool {
        self.heading_rad.is_finite() && self.reference != HeadingReference::Unknown
    }

    /// Same declared north, or both a true-north convention. Magnetic and
    /// true are different data with no variation sample here to bridge them,
    /// so they are not comparable.
    fn datum_compatible(&self, other: &Self) -> bool {
        if self.reference == HeadingReference::Unknown
            || other.reference == HeadingReference::Unknown
        {
            return false;
        }
        self.reference == other.reference
            || (is_true_north(self.reference) && is_true_north(other.reference))
    }

    fn difference(&self, other: &Self) -> f32 {
        shortest_angle_rad(self.heading_rad, other.heading_rad).abs()
    }
}

/// A datum-qualified altitude sample compared in meters. Compatibility is
/// decided by reference class and its identity (origin for local-relative,
/// geoid model for geometric MSL) — numeric equality across references is
/// meaningless and never compared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceAltitude {
    /// Altitude sample in meters.
    pub value_m: f32,
    /// The reference class the value is measured against.
    pub class: AltitudeClass,
    /// Local-origin identity for local-relative altitude.
    pub origin: OriginId,
    /// Geoid-model identity for geometric-MSL altitude.
    pub model: GeoidModelId,
}

impl Comparable for SourceAltitude {
    fn well_formed(&self) -> bool {
        if !self.value_m.is_finite() {
            return false;
        }
        match self.class {
            AltitudeClass::Unknown => false,
            AltitudeClass::GeometricMsl => self.model != GeoidModelId::UNDECLARED,
            _ => true,
        }
    }

    fn datum_compatible(&self, other: &Self) -> bool {
        if self.class != other.class || self.class == AltitudeClass::Unknown {
            return false;
        }
        match self.class {
            AltitudeClass::LocalRelative => self.origin == other.origin,
            AltitudeClass::GeometricMsl => {
                self.model == other.model && self.model != GeoidModelId::UNDECLARED
            }
            _ => true,
        }
    }

    fn difference(&self, other: &Self) -> f32 {
        (self.value_m - other.value_m).abs()
    }
}

/// Exactly one international knot in meters per second (1852 m per hour).
const KT_TO_MPS: f32 = 1852.0 / 3600.0;

/// Units for an explicit scalar or vector quantity. Zero-cost tag that keeps
/// two samples in different units from ever being compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarUnit {
    /// Meters.
    Meters,
    /// Meters per second.
    MetersPerSecond,
    /// Knots.
    Knots,
    /// The unit was not declared; the sample is ill-formed.
    Unknown,
}

impl ScalarUnit {
    /// How many canonical SI units (meters for a length, meters/second for a
    /// speed) one of this unit is. A difference is multiplied by this so the
    /// policy thresholds — stated in the canonical unit — apply identically
    /// to a knots pair and a meters-per-second pair. `Unknown` has no
    /// canonical expression and maps to infinity, so a difference involving
    /// it can never read as agreement.
    const fn canonical_factor(self) -> f32 {
        match self {
            Self::Meters | Self::MetersPerSecond => 1.0,
            Self::Knots => KT_TO_MPS,
            Self::Unknown => f32::INFINITY,
        }
    }
}

/// An explicit scalar sample (airspeed, a distance) compared by absolute
/// difference within one declared unit, expressed canonically (meters or
/// meters/second).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarMeasure {
    /// The value in `unit`.
    pub value: f32,
    /// The declared unit.
    pub unit: ScalarUnit,
}

impl Comparable for ScalarMeasure {
    fn well_formed(&self) -> bool {
        self.value.is_finite() && self.unit != ScalarUnit::Unknown
    }

    fn datum_compatible(&self, other: &Self) -> bool {
        self.unit == other.unit && self.unit != ScalarUnit::Unknown
    }

    /// The absolute difference in the canonical unit, so a knots pair is
    /// judged against the same policy thresholds as a meters-per-second pair.
    fn difference(&self, other: &Self) -> f32 {
        // Unreachable for a well-formed, datum-compatible pair; an infinite
        // difference never agrees.
        if self.unit != other.unit || self.unit == ScalarUnit::Unknown {
            return f32::INFINITY;
        }
        (self.value - other.value).abs() * self.unit.canonical_factor()
    }
}

/// An explicit three-vector sample (a velocity, a position) compared by the
/// Euclidean norm of the difference within one declared unit, expressed
/// canonically (meters or meters/second).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorMeasure {
    /// The vector components in `unit`.
    pub value: [f32; 3],
    /// The declared unit.
    pub unit: ScalarUnit,
}

impl Comparable for VectorMeasure {
    fn well_formed(&self) -> bool {
        self.unit != ScalarUnit::Unknown && self.value.iter().all(|component| component.is_finite())
    }

    fn datum_compatible(&self, other: &Self) -> bool {
        self.unit == other.unit && self.unit != ScalarUnit::Unknown
    }

    /// The norm of the difference in the canonical unit, so a knots pair is
    /// judged against the same policy thresholds as a meters-per-second pair.
    fn difference(&self, other: &Self) -> f32 {
        // Unreachable for a well-formed, datum-compatible pair; an infinite
        // difference never agrees.
        if self.unit != other.unit || self.unit == ScalarUnit::Unknown {
            return f32::INFINITY;
        }
        let dx = self.value[0] - other.value[0];
        let dy = self.value[1] - other.value[1];
        let dz = self.value[2] - other.value[2];
        sqrtf(dx * dx + dy * dy + dz * dz) * self.unit.canonical_factor()
    }
}
