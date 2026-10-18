//! Translation of a MAVLink `MAV_RESULT` into the protocol's typed
//! command outcome, so the code never crosses the adapter boundary
//! (ADR-0008).

use pilotage_adapter_api::CommandOutcome;

/// The typed outcome a `COMMAND_ACK` result code carries.
///
/// A code this table does not know reads as [`CommandOutcome::Failed`]:
/// it is not an acceptance, and an unknown answer must not move the
/// operator's lever as if it were one.
#[must_use]
pub const fn command_outcome(mav_result: u8) -> CommandOutcome {
    match mav_result {
        0 => CommandOutcome::Accepted,
        1 => CommandOutcome::TemporarilyRejected,
        2 => CommandOutcome::Denied,
        // UNSUPPORTED, COMMAND_LONG_ONLY, COMMAND_INT_ONLY, and
        // COMMAND_UNSUPPORTED_MAV_FRAME: the command form is not supported.
        3 | 7 | 8 | 9 => CommandOutcome::Unsupported,
        5 => CommandOutcome::InProgress,
        6 => CommandOutcome::Cancelled,
        // NOT_IN_CONTROL: another controller owns the vehicle.
        10 => CommandOutcome::Denied,
        _ => CommandOutcome::Failed,
    }
}

#[cfg(test)]
mod tests;
