//! Flight-director commands (FD-01, #261).
//!
//! A director is a commanded attitude to fly, produced by an autopilot
//! or flight computer upstream — never synthesised on the display side
//! from deviation, which would put an autopilot function in the
//! presentation layer. The vocabulary carries the commanded pitch and
//! roll together with the mode and engagement that say WHAT is
//! commanding them: a command bar without its mode is not
//! interpretable, so the annunciation is part of the contract, not a
//! follow-up.
//!
//! Fail-closed throughout: an unknown mode or engagement decodes to
//! the `Unknown` sentinel and the director fails rather than
//! commanding; command bars disappear entirely under degradation — a
//! frozen or dashed command is still a command.
//!
//! No shipped posture publishes this group yet: until a feeder wires
//! an upstream autopilot, the group resolves `Missing` everywhere and
//! the display draws nothing — the contract is live, the data is not.

/// What is producing the commanded attitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FdMode {
    /// Hold the commanded attitude.
    AttitudeHold,
    /// Lateral/vertical navigation tracking.
    Nav,
    /// Approach tracking.
    Approach,
    /// The wire carried a mode this build does not know, or none was
    /// declared; the director fails rather than guessing.
    #[default]
    Unknown,
}

impl FdMode {
    /// Fail-closed wire decoding.
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::AttitudeHold,
            1 => Self::Nav,
            2 => Self::Approach,
            _ => Self::Unknown,
        }
    }

    /// Wire encoding; `Unknown` round-trips as unknown.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::AttitudeHold => 0,
            Self::Nav => 1,
            Self::Approach => 2,
            Self::Unknown => 255,
        }
    }

    /// The annunciation label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AttitudeHold => "FD ATT",
            Self::Nav => "FD NAV",
            Self::Approach => "FD APR",
            Self::Unknown => "FD",
        }
    }
}

/// Whether the director is commanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FdEngagement {
    /// Not commanding; bars are not drawn.
    Off,
    /// Armed to engage; annunciated, bars not drawn.
    Armed,
    /// Commanding; bars are drawn.
    Engaged,
    /// The wire carried an engagement this build does not know; the
    /// director fails rather than guessing.
    #[default]
    Unknown,
}

impl FdEngagement {
    /// Fail-closed wire decoding.
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Armed,
            2 => Self::Engaged,
            _ => Self::Unknown,
        }
    }

    /// Wire encoding; `Unknown` round-trips as unknown.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Armed => 1,
            Self::Engaged => 2,
            Self::Unknown => 255,
        }
    }
}

/// One flight-director sample: the commanded attitude and what
/// commands it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FdSample {
    /// Commanded pitch, radians, positive nose-up.
    pub pitch_cmd_rad: f32,
    /// Commanded roll, radians, positive right-wing-down.
    pub roll_cmd_rad: f32,
    /// What produces the command.
    pub mode: FdMode,
    /// Whether the director is commanding.
    pub engagement: FdEngagement,
}
