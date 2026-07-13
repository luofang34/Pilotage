//! Typed turn and slip/skid inputs (DYN-01).
//!
//! Body-axis yaw rate is not a turn indication: at nonzero roll or
//! pitch, body `r`, heading rate, track rate, and rotation about local
//! vertical are different quantities. The turn vocabulary here makes
//! the basis explicit and gives body rate NO representation — a feeder
//! cannot label body `r` as a turn rate even by mistake, and the
//! display never derives one from attitude rates.
//!
//! Axes and signs: turn rate is positive turning RIGHT (clockwise from
//! above), radians/second about the local vertical per the declared
//! basis. Slip/skid input is lateral specific force along body +Y
//! (right), meters/second²; the coordination ball displaces OPPOSITE
//! the lateral specific force, so a positive (rightward) force shows
//! the ball deflected LEFT. Both conventions are symmetric and pinned
//! by tests.

/// The quantity a turn-rate sample actually measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnBasis {
    /// Rate of change of heading (from the heading source's reference).
    HeadingRate,
    /// Rate of change of ground track.
    TrackRate,
    /// The wire carried a basis this build does not know, or none was
    /// declared; the turn indication fails rather than guessing.
    #[default]
    Unknown,
}

impl TurnBasis {
    /// Fail-closed wire decoding.
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::HeadingRate,
            1 => Self::TrackRate,
            _ => Self::Unknown,
        }
    }

    /// Wire encoding; `Unknown` round-trips as unknown.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::HeadingRate => 0,
            Self::TrackRate => 1,
            Self::Unknown => 255,
        }
    }

    /// The cue label identifying this basis to the pilot.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::HeadingRate => "HDG",
            Self::TrackRate => "TRK",
            Self::Unknown => "REF",
        }
    }
}

/// One turn-rate sample with its declared basis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnSample {
    /// Turn rate in radians/second, positive turning right.
    pub rate_rps: f32,
    /// What the rate measures.
    pub basis: TurnBasis,
}

/// The dynamics estimate group: turn and coordination, independently
/// optional because a vehicle may provide either without the other.
/// Missing slip stays missing — it is never synthesized centered.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DynSample {
    /// Turn-rate sample, when the upstream estimator provides one.
    pub turn: Option<TurnSample>,
    /// Lateral specific force along body +Y (right), m/s²; the slip
    /// ball displaces opposite this force.
    pub lateral_mps2: Option<f32>,
}

#[cfg(test)]
mod tests;
