//! The heading and altitude intent: what a vehicle that holds heading,
//! altitude, and airspeed with its own sensors is asked to hold.

/// The reference an intent altitude is measured against.
///
/// The wire codes are the vertical-datum codes of a geodetic fix, so the
/// two vocabularies cannot name one reference in two ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AltitudeReference {
    /// Height above the WGS-84 ellipsoid.
    Ellipsoid,
    /// Height above the local origin of the flight controller: the origin
    /// its local position refers to.
    LocalOrigin,
}

impl AltitudeReference {
    /// The wire code of the reference.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Ellipsoid => 1,
            Self::LocalOrigin => 6,
        }
    }

    /// The reference of a wire code, or `None` for a code that names no
    /// reference an intent altitude can have.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::Ellipsoid),
            6 => Some(Self::LocalOrigin),
            _ => None,
        }
    }
}

/// A true heading and an altitude to hold, and optionally an airspeed.
///
/// The vehicle holds each of them with its own sensors, so it needs no
/// position estimate to fly this intent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadingAltitudeIntent {
    /// True heading, radians in `[-π, π]`: zero north, positive toward
    /// east.
    pub heading_rad: f32,
    /// Altitude, meters, positive up, against [`Self::reference`].
    pub altitude_m: f32,
    /// What [`Self::altitude_m`] is measured against.
    pub reference: AltitudeReference,
    /// Indicated airspeed, m/s, greater than zero. `None` leaves the
    /// vehicle at its own cruise airspeed.
    pub airspeed_mps: Option<f32>,
}
