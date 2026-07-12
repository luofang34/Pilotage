//! Typed heading references and the single sanctioned conversion path.
//!
//! Operational heading is an independent sample with an explicit
//! magnetic/true reference — never implicit quaternion yaw, which is
//! local-NED orientation, ill-conditioned near vertical pitch, and
//! carries no reference. Magnetic/true conversion happens in exactly
//! one place, [`convert_heading`], and only with a valid magnetic
//! variation sample whose sign convention (east positive), source, and
//! freshness are explicit. Nothing here ever fabricates or substitutes
//! a reference.

use core::f32::consts::{PI, TAU};

/// The north reference a horizontal angle is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadingReference {
    /// Magnetic north.
    Magnetic,
    /// True north.
    True,
    /// The simulator's local-NED north, explicitly declared by the
    /// feeder. Numerically a true-north convention for the local scene;
    /// labelled SIM so it can never pass for a navigation-grade source.
    SimLocalTrue,
    /// The wire carried a reference this build does not know; heading
    /// fails rather than guessing.
    Unknown,
}

impl HeadingReference {
    /// Fail-closed wire decoding.
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Magnetic,
            1 => Self::True,
            2 => Self::SimLocalTrue,
            _ => Self::Unknown,
        }
    }

    /// Wire encoding; `Unknown` round-trips as unknown.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Magnetic => 0,
            Self::True => 1,
            Self::SimLocalTrue => 2,
            Self::Unknown => 255,
        }
    }

    /// The HSI label identifying this reference.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Magnetic => "MAG",
            Self::True => "TRU",
            Self::SimLocalTrue => "SIM",
            Self::Unknown => "REF",
        }
    }

    /// Whether both references measure from (a) true north, so values
    /// carry across without a variation sample.
    #[must_use]
    const fn true_north(self) -> bool {
        matches!(self, Self::True | Self::SimLocalTrue)
    }
}

/// An independent heading sample with its declared reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadingSample {
    /// Heading in radians clockwise from the declared north.
    pub heading_rad: f32,
    /// Which north the heading is measured from.
    pub reference: HeadingReference,
}

/// Identity of the magnetic-variation source. Zero is "undeclared": an
/// unattributed variation must not silently rotate the compass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariationSourceId(pub u8);

impl VariationSourceId {
    /// No source declared.
    pub const UNDECLARED: Self = Self(0);
}

/// A magnetic-variation sample. Sign convention: east variation is
/// positive, so `true = magnetic + variation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagneticVariation {
    /// Variation in radians, east positive.
    pub east_positive_rad: f32,
    /// Which model/source produced it.
    pub source: VariationSourceId,
}

/// Why a reference conversion is not available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionFault {
    /// Either side's reference is unknown.
    UnknownReference,
    /// Magnetic/true conversion was required but no usable variation
    /// sample exists (absent, stale, undeclared source, or non-finite).
    VariationUnavailable,
}

/// Normalizes an angle to `[0, 2π)`.
#[must_use]
pub fn wrap_2pi(angle_rad: f32) -> f32 {
    let wrapped = angle_rad % TAU;
    if wrapped < 0.0 {
        wrapped + TAU
    } else {
        wrapped
    }
}

/// The signed minimal rotation from `from_rad` to `to_rad`, in
/// `(-π, π]`; correct across the 359°/1° wrap in both directions.
#[must_use]
pub fn shortest_angle_rad(from_rad: f32, to_rad: f32) -> f32 {
    let delta = wrap_2pi(to_rad - from_rad);
    if delta > PI { delta - TAU } else { delta }
}

/// Converts a horizontal angle between references. This is the single
/// sanctioned conversion path: same-true-north pairs carry across
/// unchanged, magnetic/true crossings require a usable variation sample
/// (east positive: `true = magnetic + variation`), and an unknown
/// reference on either side refuses. The result is wrapped to
/// `[0, 2π)`.
pub fn convert_heading(
    value_rad: f32,
    from: HeadingReference,
    to: HeadingReference,
    variation: Option<&MagneticVariation>,
) -> Result<f32, ConversionFault> {
    if from == HeadingReference::Unknown || to == HeadingReference::Unknown {
        return Err(ConversionFault::UnknownReference);
    }
    if from == to || (from.true_north() && to.true_north()) {
        return Ok(wrap_2pi(value_rad));
    }
    let variation = usable_variation(variation)?;
    let converted = if from == HeadingReference::Magnetic {
        value_rad + variation
    } else {
        value_rad - variation
    };
    Ok(wrap_2pi(converted))
}

fn usable_variation(variation: Option<&MagneticVariation>) -> Result<f32, ConversionFault> {
    match variation {
        Some(sample)
            if sample.east_positive_rad.is_finite()
                && sample.source != VariationSourceId::UNDECLARED =>
        {
            Ok(sample.east_positive_rad)
        }
        _ => Err(ConversionFault::VariationUnavailable),
    }
}

#[cfg(test)]
mod tests;
