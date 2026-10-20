//! Flight-controller landed detector vocabulary.

use crate::wire;

/// Landed state reported by the flight controller.
///
/// A command, an altitude, or elapsed time cannot supply this observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandedState {
    /// The detector does not state a usable result.
    Unknown,
    /// The detector reports the vehicle on the ground.
    OnGround,
    /// The detector reports the vehicle in the air.
    InAir,
    /// The detector reports takeoff in progress.
    TakingOff,
    /// The detector reports landing in progress.
    Landing,
}

impl LandedState {
    /// Returns the wire value for this state.
    #[must_use]
    pub const fn to_wire(self) -> wire::LandedState {
        match self {
            Self::Unknown => wire::LandedState::Unspecified,
            Self::OnGround => wire::LandedState::OnGround,
            Self::InAir => wire::LandedState::InAir,
            Self::TakingOff => wire::LandedState::TakingOff,
            Self::Landing => wire::LandedState::Landing,
        }
    }

    /// Reads a wire value. Unrecognized values provide no usable state.
    #[must_use]
    pub fn from_wire(raw: i32) -> Self {
        match wire::LandedState::try_from(raw) {
            Ok(wire::LandedState::OnGround) => Self::OnGround,
            Ok(wire::LandedState::InAir) => Self::InAir,
            Ok(wire::LandedState::TakingOff) => Self::TakingOff,
            Ok(wire::LandedState::Landing) => Self::Landing,
            Ok(wire::LandedState::Unspecified) | Err(_) => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests;
