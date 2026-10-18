//! The vehicle's typed answer to a discrete command, independent of the
//! flight-controller family (`pilotage.v1.CommandOutcome`).
//!
//! An adapter translates its flight controller's own result codes into
//! this vocabulary, so no client decodes them.

use crate::wire;

/// How long a host has to answer an arm or disarm, in milliseconds.
///
/// The window is part of the protocol, not of one flight controller: an
/// adapter that sequences a command over a longer time (a stream that
/// must flow before the arm goes out) reports
/// [`CommandOutcome::InProgress`], and each new `InProgress` starts the
/// window again. The value covers a slow link plus one flight-controller
/// answer.
pub const COMMAND_ANSWER_WINDOW_MS: u32 = 5000;

/// The vehicle's answer to one discrete command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandOutcome {
    /// The vehicle accepted the command.
    Accepted,
    /// The vehicle works on the command and will answer again.
    InProgress,
    /// The vehicle cannot do the command now, but it can be possible
    /// later (a failed preflight check is the usual cause).
    TemporarilyRejected,
    /// The vehicle refused the command.
    Denied,
    /// The vehicle does not support the command.
    Unsupported,
    /// The vehicle tried the command and it failed.
    Failed,
    /// The vehicle stopped the command before it was complete.
    Cancelled,
}

impl CommandOutcome {
    /// Whether this answer says the vehicle will not do the command.
    /// `InProgress` is not a refusal: an answer is still to come.
    #[must_use]
    pub const fn is_refusal(self) -> bool {
        !matches!(self, Self::Accepted | Self::InProgress)
    }

    /// The wire enum for this outcome.
    #[must_use]
    pub const fn to_wire(self) -> wire::CommandOutcome {
        match self {
            Self::Accepted => wire::CommandOutcome::Accepted,
            Self::InProgress => wire::CommandOutcome::InProgress,
            Self::TemporarilyRejected => wire::CommandOutcome::TemporarilyRejected,
            Self::Denied => wire::CommandOutcome::Denied,
            Self::Unsupported => wire::CommandOutcome::Unsupported,
            Self::Failed => wire::CommandOutcome::Failed,
            Self::Cancelled => wire::CommandOutcome::Cancelled,
        }
    }

    /// The outcome a raw wire value carries. `None` for
    /// `COMMAND_OUTCOME_UNSPECIFIED` and for a value this build does not
    /// know: neither is an answer, and neither may read as accepted.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::CommandOutcome::try_from(raw).ok()? {
            wire::CommandOutcome::Unspecified => None,
            wire::CommandOutcome::Accepted => Some(Self::Accepted),
            wire::CommandOutcome::InProgress => Some(Self::InProgress),
            wire::CommandOutcome::TemporarilyRejected => Some(Self::TemporarilyRejected),
            wire::CommandOutcome::Denied => Some(Self::Denied),
            wire::CommandOutcome::Unsupported => Some(Self::Unsupported),
            wire::CommandOutcome::Failed => Some(Self::Failed),
            wire::CommandOutcome::Cancelled => Some(Self::Cancelled),
        }
    }
}

#[cfg(test)]
mod tests;
