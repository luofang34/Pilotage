//! FC-state freshness, fail closed.
//!
//! A report is accepted only when its COMPLETE stamp validates for the
//! FC-state role; the source identity (id + incarnation) is pinned at
//! first acceptance for the session; the epoch/sequence pair must
//! strictly ADVANCE in wrapping serial order — duplicates and
//! reordered/older reports never refresh age and never regress the
//! displayed state; and the arm value itself must be in range.
//! Heartbeat loss surfaces as stale instead of a forever-fresh arm
//! state.

use crate::stamp::{ROLE_FC_STATE, RawStamp, serial_is_newer, stamp_fault_for_role};

/// The FC's arm/disarm COMMAND_ACK verdict riding the fc-state lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FcCommand {
    /// Whether the acknowledged command was an arm (true) or disarm.
    pub arm: bool,
    /// The FC's result code.
    pub result: u32,
}

/// One decoded fc-state report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FcReport {
    /// The report's stamp.
    pub stamp: RawStamp,
    /// Arm-state code (0–2).
    pub arm_state: u32,
    /// The last command verdict, when the wire carried a well-formed
    /// one. A malformed verdict degrades to `None` rather than
    /// rejecting the whole report: the arm state itself is still valid
    /// and fresh.
    pub last_command: Option<FcCommand>,
}

/// The display view of the newest accepted report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FcView {
    /// Arm-state code.
    pub arm_state: u32,
    /// The last command verdict, if any.
    pub last_command: Option<FcCommand>,
    /// Milliseconds since the newest report, on the caller's clock.
    pub age_ms: f64,
    /// The age exceeds the staleness threshold.
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Last {
    arm_state: u32,
    last_command: Option<FcCommand>,
    source_id: u64,
    incarnation: [u8; 16],
    epoch: u32,
    sequence: u32,
    first_seen_ms: f64,
}

/// Tracks the single pinned FC report stream.
#[derive(Debug)]
pub struct FcStateTracker {
    stale_after_ms: f64,
    last: Option<Last>,
}

impl Default for FcStateTracker {
    fn default() -> Self {
        Self::new(3000.0)
    }
}

impl FcStateTracker {
    /// A tracker with the given staleness threshold in milliseconds.
    pub fn new(stale_after_ms: f64) -> Self {
        Self {
            stale_after_ms,
            last: None,
        }
    }

    /// Feeds one decoded fc-state report (or `None`) and returns the
    /// current view. Only a NEW report — pinned identity, epoch
    /// advanced, or same epoch with the sequence strictly newer in
    /// wrapping order — restarts the age clock.
    pub fn observe(&mut self, report: Option<&FcReport>, now_ms: f64) -> Option<FcView> {
        if let Some(report) = report
            && self.accepts(report)
        {
            self.last = Some(Last {
                arm_state: report.arm_state,
                last_command: report.last_command,
                source_id: report.stamp.source_id,
                incarnation: report.stamp.incarnation,
                epoch: report.stamp.epoch,
                sequence: report.stamp.sequence,
                first_seen_ms: now_ms,
            });
        }
        self.view(now_ms)
    }

    /// Whether a report is a valid, strictly-new observation from the
    /// pinned source. Every rejection is fail-closed: the previous view
    /// (and its age) stands.
    pub fn accepts(&self, report: &FcReport) -> bool {
        if stamp_fault_for_role(&report.stamp, ROLE_FC_STATE).is_some() {
            return false;
        }
        if report.arm_state > 2 {
            return false;
        }
        let Some(last) = &self.last else {
            return true;
        };
        let stamp = &report.stamp;
        // Identity is pinned for the session: a different source id or
        // incarnation is not this FC's report stream.
        if stamp.source_id != last.source_id || stamp.incarnation != last.incarnation {
            return false;
        }
        if stamp.epoch == last.epoch {
            return serial_is_newer(stamp.sequence, last.sequence);
        }
        // A newer epoch (FC restart/re-attach) restarts the numbering;
        // an older epoch is a replay.
        serial_is_newer(stamp.epoch, last.epoch)
    }

    /// The display view: `None` before any report; stale once the
    /// newest report's age exceeds the threshold.
    pub fn view(&self, now_ms: f64) -> Option<FcView> {
        self.last.as_ref().map(|last| {
            let age_ms = now_ms - last.first_seen_ms;
            FcView {
                arm_state: last.arm_state,
                last_command: last.last_command,
                age_ms,
                stale: age_ms > self.stale_after_ms,
            }
        })
    }
}

#[cfg(test)]
mod tests;
