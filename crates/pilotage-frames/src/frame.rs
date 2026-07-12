//! Typed reference-frame identities and their fail-closed wire codec.

use crate::error::FrameError;

/// A reference frame a kinematic quantity can be expressed in.
///
/// Identity is explicit everywhere: no canonical attitude, pose,
/// velocity, or rate carries an implicit frame. The set covers aircraft
/// today and orbital vehicles later; none of these imply a propagation
/// model — a transform between them is always *supplied*, never derived
/// here (orbital propagation is out of scope by contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameId {
    /// Vehicle body frame (aircraft convention FRD when paired with NED).
    Body = 0,
    /// Sensor/installation frame: a fixed mount relative to the body.
    Installation = 1,
    /// Local navigation frame: north-east-down at a declared origin.
    Ned = 2,
    /// Earth-centered, Earth-fixed.
    Ecef = 3,
    /// Earth-centered inertial (a declared realization; the epoch on the
    /// carrying type says which instant pins it).
    Eci = 4,
    /// Local-vertical/local-horizontal orbit frame.
    Lvlh = 5,
    /// Radial/transverse/normal orbit frame.
    Rtn = 6,
    /// Relative to a declared target vehicle or feature.
    TargetRelative = 7,
}

impl FrameId {
    /// Wire encoding.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, failing closed: a frame this build cannot
    /// place has no benign fallback — composing through a guessed frame
    /// would silently relabel geometry.
    ///
    /// # Errors
    ///
    /// [`FrameError::UnknownFrame`] for any byte outside the known set.
    pub const fn from_u8(code: u8) -> Result<Self, FrameError> {
        match code {
            0 => Ok(Self::Body),
            1 => Ok(Self::Installation),
            2 => Ok(Self::Ned),
            3 => Ok(Self::Ecef),
            4 => Ok(Self::Eci),
            5 => Ok(Self::Lvlh),
            6 => Ok(Self::Rtn),
            7 => Ok(Self::TargetRelative),
            _ => Err(FrameError::UnknownFrame { code }),
        }
    }
}

#[cfg(test)]
mod tests;
