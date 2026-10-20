//! Flight-controller motor operation reports.

use crate::wire;

/// Motor operation as the flight controller reports it.
///
/// This is separate from arm state. It does not measure rotor speed or prove
/// that the rotors are physically still.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorState {
    /// No usable motor operation report is available.
    Unknown,
    /// The flight controller reports the motors stopped.
    Stopped,
    /// The flight controller reports the motors running.
    Running,
}

impl MotorState {
    /// Returns the wire value for this report.
    #[must_use]
    pub const fn to_wire(self) -> wire::MotorState {
        match self {
            Self::Unknown => wire::MotorState::Unspecified,
            Self::Stopped => wire::MotorState::Stopped,
            Self::Running => wire::MotorState::Running,
        }
    }

    /// Reads a wire value. Unspecified and unrecognized values remain unknown.
    #[must_use]
    pub fn from_wire(raw: i32) -> Self {
        match wire::MotorState::try_from(raw) {
            Ok(wire::MotorState::Stopped) => Self::Stopped,
            Ok(wire::MotorState::Running) => Self::Running,
            Ok(wire::MotorState::Unspecified) | Err(_) => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests;
