//! FC-owned reports cached outside the estimate measurement discipline:
//! the flight mode, the verdict on each command, and recent status text.
//! None of them is an estimate, and none can authorize one.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use super::CommandAckReport;
use crate::codec::STATUS_TEXT_LEN;

/// How many status texts the link keeps. An FC that refuses an arm sends
/// its reasons in a burst next to the refusal, so a short history holds
/// them while the adapter pairs them with the verdict.
pub const STATUS_TEXT_HISTORY: usize = 16;

/// One STATUSTEXT, decoded to text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusTextReport {
    /// MAV_SEVERITY (0 emergency .. 7 debug).
    pub severity: u8,
    /// The text up to its first NUL, with invalid UTF-8 replaced.
    pub text: String,
    /// Wrapping count of status texts received on this link.
    pub sequence: u32,
    /// When the text was received.
    pub received_at: Instant,
}

impl StatusTextReport {
    pub(super) fn decode(
        severity: u8,
        raw: &[u8; STATUS_TEXT_LEN],
        sequence: u32,
        received_at: Instant,
    ) -> Self {
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        Self {
            severity,
            text: String::from_utf8_lossy(raw.get(..end).unwrap_or(&[])).into_owned(),
            sequence,
            received_at,
        }
    }
}

/// FC-owned reports from the selected source.
#[derive(Debug, Default)]
pub struct FcReports {
    /// HEARTBEAT `custom_mode` of the last FC heartbeat.
    pub custom_mode: Option<u32>,
    /// HEARTBEAT `autopilot` of the last FC heartbeat.
    pub autopilot: Option<u8>,
    /// The latest acknowledgement of each command id. One slot per
    /// command, because an FC acknowledges every periodic message-interval
    /// request: a single latest-ack slot would bury an arm or mode verdict
    /// within a second.
    pub command_acks: BTreeMap<u16, CommandAckReport>,
    /// The most recent status texts, oldest first, at most
    /// [`STATUS_TEXT_HISTORY`].
    pub status_texts: VecDeque<StatusTextReport>,
}

impl FcReports {
    /// The latest acknowledgement of `command`, if one arrived.
    #[must_use]
    pub fn command_ack(&self, command: u16) -> Option<CommandAckReport> {
        self.command_acks.get(&command).copied()
    }

    pub(super) fn push_status_text(
        &mut self,
        severity: u8,
        raw: &[u8; STATUS_TEXT_LEN],
        now: Instant,
    ) {
        let sequence = self
            .status_texts
            .back()
            .map_or(0, |last| last.sequence.wrapping_add(1));
        if self.status_texts.len() == STATUS_TEXT_HISTORY {
            self.status_texts.pop_front();
        }
        self.status_texts
            .push_back(StatusTextReport::decode(severity, raw, sequence, now));
    }
}

#[cfg(test)]
mod tests;
