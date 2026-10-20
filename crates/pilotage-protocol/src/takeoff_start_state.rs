//! The required vehicle state when a native takeoff procedure starts.

use crate::wire;

/// The start state an adapter requires for its native takeoff procedure.
///
/// This is a requirement, not a measurement or permission to start. The
/// adapter must check all start conditions when it receives the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TakeoffStartState {
    /// The vehicle must be armed and on the ground.
    ArmedOnGround,
    /// The vehicle must be on the ground with an FC report of stopped motors.
    ///
    /// A disarmed report is not proof that the motors have stopped.
    MotorsStoppedOnGround,
    /// The FC must report stopped motors and check its other start conditions.
    ///
    /// This requirement does not assert that a ground state was measured.
    MotorsStopped,
}

impl TakeoffStartState {
    /// Returns the wire value for this requirement.
    #[must_use]
    pub const fn to_wire(self) -> wire::TakeoffStartState {
        match self {
            Self::ArmedOnGround => wire::TakeoffStartState::ArmedOnGround,
            Self::MotorsStoppedOnGround => wire::TakeoffStartState::MotorsStoppedOnGround,
            Self::MotorsStopped => wire::TakeoffStartState::MotorsStopped,
        }
    }

    /// Returns `UNSPECIFIED` when the requirement is unknown.
    #[must_use]
    pub const fn option_to_wire(state: Option<Self>) -> wire::TakeoffStartState {
        match state {
            Some(state) => state.to_wire(),
            None => wire::TakeoffStartState::Unspecified,
        }
    }

    /// Reads a wire value. Unspecified and unrecognized values remain unknown.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::TakeoffStartState::try_from(raw).ok()? {
            wire::TakeoffStartState::Unspecified => None,
            wire::TakeoffStartState::ArmedOnGround => Some(Self::ArmedOnGround),
            wire::TakeoffStartState::MotorsStoppedOnGround => Some(Self::MotorsStoppedOnGround),
            wire::TakeoffStartState::MotorsStopped => Some(Self::MotorsStopped),
        }
    }
}

#[cfg(test)]
mod tests;
