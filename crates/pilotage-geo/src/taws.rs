//! Terrain-awareness alerts as an INDEPENDENT input.
//!
//! Synthetic vision must never become an implicit terrain-awareness-and-warning
//! system. A [`TawsAlert`] is a separate input with its own source identity and
//! epoch; nothing in this crate derives a TAWS alert from the SVS scene or
//! folds SVS availability into a terrain hazard. The two are independent values
//! with independent failure behavior: an available scene asserts nothing about
//! terrain clearance, and a TAWS warning is not suppressed by a degraded scene.
//! A real display composes them side by side; it does not let one stand in for
//! the other.

use crate::identity::SourceStamp;

/// The terrain hazard an independent TAWS reports. `None` is not an assurance of
/// clearance — it is the absence of an alert from the TAWS input, distinct from
/// anything the SVS scene shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TawsHazard {
    /// No active terrain alert from the TAWS input.
    None = 0,
    /// Terrain caution.
    Caution = 1,
    /// Terrain warning (e.g. pull-up).
    Warning = 2,
}

impl TawsHazard {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::None),
            1 => Some(Self::Caution),
            2 => Some(Self::Warning),
            _ => None,
        }
    }
}

/// An independent terrain-awareness alert, carrying its own source stamp. It is
/// an input to a display, never an output of synthetic vision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TawsAlert {
    /// The reported terrain hazard.
    pub hazard: TawsHazard,
    /// Identity, epoch, and integrity of the TAWS source (separate from any
    /// SVS position/attitude source).
    pub stamp: SourceStamp,
}

#[cfg(test)]
mod tests;
