//! Flight procedure progress, separate from command acceptance and FC state.

use crate::wire;

/// A discrete procedure whose completion needs vehicle observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightProcedureKind {
    /// The vehicle's native takeoff procedure.
    Takeoff,
    /// The vehicle's native landing procedure.
    Land,
}

impl FlightProcedureKind {
    /// The wire value for this procedure.
    #[must_use]
    pub const fn to_wire(self) -> wire::FlightProcedureKind {
        match self {
            Self::Takeoff => wire::FlightProcedureKind::Takeoff,
            Self::Land => wire::FlightProcedureKind::Land,
        }
    }

    /// A known procedure, or `None` for an absent or unknown wire value.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::FlightProcedureKind::try_from(raw).ok()? {
            wire::FlightProcedureKind::Unspecified => None,
            wire::FlightProcedureKind::Takeoff => Some(Self::Takeoff),
            wire::FlightProcedureKind::Land => Some(Self::Land),
        }
    }
}

/// Progress established by the adapter's execution and observation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightProcedurePhase {
    /// The local command queue accepted the request.
    Queued,
    /// The transport confirmed a complete command write.
    Written,
    /// The FC acknowledged the request or the adapter observed progress.
    InProgress,
    /// Fresh vehicle observations established the completion condition.
    Completed,
    /// The procedure cannot complete under its current conditions.
    Failed,
    /// Completion was not observed before the deadline; motion can continue.
    TimedOut,
    /// The adapter ended the procedure; this does not confirm a vehicle stop.
    Cancelled,
    /// Transport loss prevents a conclusive result. The request must not be replayed.
    OutcomeUnknown,
}

impl FlightProcedurePhase {
    /// Whether this status ends this request's procedure tracking.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::TimedOut
                | Self::Cancelled
                | Self::OutcomeUnknown
        )
    }

    /// The wire value for this phase.
    #[must_use]
    pub const fn to_wire(self) -> wire::FlightProcedurePhase {
        match self {
            Self::Queued => wire::FlightProcedurePhase::Queued,
            Self::Written => wire::FlightProcedurePhase::Written,
            Self::InProgress => wire::FlightProcedurePhase::InProgress,
            Self::Completed => wire::FlightProcedurePhase::Completed,
            Self::Failed => wire::FlightProcedurePhase::Failed,
            Self::TimedOut => wire::FlightProcedurePhase::TimedOut,
            Self::Cancelled => wire::FlightProcedurePhase::Cancelled,
            Self::OutcomeUnknown => wire::FlightProcedurePhase::OutcomeUnknown,
        }
    }

    /// A known phase, or `None` for an absent or unknown wire value.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::FlightProcedurePhase::try_from(raw).ok()? {
            wire::FlightProcedurePhase::Unspecified => None,
            wire::FlightProcedurePhase::Queued => Some(Self::Queued),
            wire::FlightProcedurePhase::Written => Some(Self::Written),
            wire::FlightProcedurePhase::InProgress => Some(Self::InProgress),
            wire::FlightProcedurePhase::Completed => Some(Self::Completed),
            wire::FlightProcedurePhase::Failed => Some(Self::Failed),
            wire::FlightProcedurePhase::TimedOut => Some(Self::TimedOut),
            wire::FlightProcedurePhase::Cancelled => Some(Self::Cancelled),
            wire::FlightProcedurePhase::OutcomeUnknown => Some(Self::OutcomeUnknown),
        }
    }
}

/// Why a procedure stopped or could not establish completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightProcedureReason {
    /// The flight controller refused the command.
    CommandRefused,
    /// The transport could not confirm a complete write.
    WriteFailed,
    /// The write or completion deadline expired.
    DeadlineExpired,
    /// Required vehicle measurements are absent or unusable.
    MeasurementUnavailable,
    /// The vehicle's measurement identity changed during the procedure.
    SourceChanged,
    /// The issuing authority was revoked.
    AuthorityLost,
    /// The vehicle reported itself disarmed.
    VehicleDisarmed,
    /// The vehicle left the mode required by the procedure.
    ModeChanged,
    /// Another explicit operation interrupted this procedure.
    Interrupted,
}

impl FlightProcedureReason {
    /// The wire value for this cause.
    #[must_use]
    pub const fn to_wire(self) -> wire::FlightProcedureReason {
        match self {
            Self::CommandRefused => wire::FlightProcedureReason::CommandRefused,
            Self::WriteFailed => wire::FlightProcedureReason::WriteFailed,
            Self::DeadlineExpired => wire::FlightProcedureReason::DeadlineExpired,
            Self::MeasurementUnavailable => wire::FlightProcedureReason::MeasurementUnavailable,
            Self::SourceChanged => wire::FlightProcedureReason::SourceChanged,
            Self::AuthorityLost => wire::FlightProcedureReason::AuthorityLost,
            Self::VehicleDisarmed => wire::FlightProcedureReason::VehicleDisarmed,
            Self::ModeChanged => wire::FlightProcedureReason::ModeChanged,
            Self::Interrupted => wire::FlightProcedureReason::Interrupted,
        }
    }

    /// A known cause, or `None` for an absent or unknown wire value.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::FlightProcedureReason::try_from(raw).ok()? {
            wire::FlightProcedureReason::Unspecified => None,
            wire::FlightProcedureReason::CommandRefused => Some(Self::CommandRefused),
            wire::FlightProcedureReason::WriteFailed => Some(Self::WriteFailed),
            wire::FlightProcedureReason::DeadlineExpired => Some(Self::DeadlineExpired),
            wire::FlightProcedureReason::MeasurementUnavailable => {
                Some(Self::MeasurementUnavailable)
            }
            wire::FlightProcedureReason::SourceChanged => Some(Self::SourceChanged),
            wire::FlightProcedureReason::AuthorityLost => Some(Self::AuthorityLost),
            wire::FlightProcedureReason::VehicleDisarmed => Some(Self::VehicleDisarmed),
            wire::FlightProcedureReason::ModeChanged => Some(Self::ModeChanged),
            wire::FlightProcedureReason::Interrupted => Some(Self::Interrupted),
        }
    }
}

#[cfg(test)]
mod tests;
