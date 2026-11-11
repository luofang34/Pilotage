//! The receipt for a mission's arm comes from the vehicle, not from the
//! command path.
//!
//! An adapter that accepts an arm action has only sent the command. The
//! flight controller can still refuse it, for example when a preflight
//! check fails. So the host gives the mission engine its arm receipt only
//! when the flight controller answers: it reports itself armed (the
//! heartbeat), or its acknowledgement refuses the arm (COMMAND_ACK), or no
//! answer comes before the confirmation timeout. A refused or unanswered
//! arm is a refused receipt, and the engine retries the arm under its own
//! retry limit.

use pilotage_protocol::{CommandOutcome, wire};
use tracing::info;

/// The flight controller's answer to an arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArmVerdict {
    /// The flight controller reports itself armed.
    Armed,
    /// The flight controller's acknowledgement refuses the arm.
    Refused(CommandOutcome),
    /// No answer came before the confirmation timeout.
    Unanswered,
}

impl ArmVerdict {
    /// Whether the verdict is an accepted receipt.
    pub(super) const fn armed(self) -> bool {
        matches!(self, Self::Armed)
    }
}

/// One arm that waits for the vehicle's answer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArmReceipt {
    pending: Option<Pending>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    mission_id: u64,
    deadline_ns: u64,
}

impl ArmReceipt {
    /// Starts to wait for the answer to the arm with the engine's action
    /// id `mission_id`, sent at `now_ns`, until `now_ns + timeout_ns`.
    pub(super) fn await_vehicle(&mut self, mission_id: u64, now_ns: u64, timeout_ns: u64) {
        self.pending = Some(Pending {
            mission_id,
            deadline_ns: now_ns.saturating_add(timeout_ns),
        });
    }

    /// The answer that one flight-controller report gives, if any. A
    /// stamped arm state of armed accepts; an acknowledgement of an arm
    /// that refuses it refuses.
    pub(super) fn observe(&mut self, fc: &wire::FcState) -> Option<(u64, ArmVerdict)> {
        let pending = self.pending?;
        let armed = fc.stamp.is_some() && fc.arm_state == 2;
        let refusal = (fc.last_command_kind == 1)
            .then(|| CommandOutcome::from_wire(fc.last_command_outcome))
            .flatten()
            .filter(|outcome| outcome.is_refusal());
        let verdict = if armed {
            ArmVerdict::Armed
        } else {
            ArmVerdict::Refused(refusal?)
        };
        self.pending = None;
        Some((pending.mission_id, verdict))
    }

    /// The unanswered arm, once its timeout has passed at `now_ns`.
    pub(super) fn expire(&mut self, now_ns: u64) -> Option<(u64, ArmVerdict)> {
        let pending = self
            .pending
            .filter(|pending| now_ns >= pending.deadline_ns)?;
        self.pending = None;
        Some((pending.mission_id, ArmVerdict::Unanswered))
    }
}

impl super::MissionTask {
    /// Gives the engine the arm receipt that the flight controller's
    /// answer decided.
    pub(super) fn settle_arm(&mut self, mission_id: u64, verdict: ArmVerdict) {
        info!(?verdict, "the flight controller answered the mission arm");
        if let Some(mission) = self.mission.as_mut() {
            mission.on_action_result(mission_id, verdict.armed());
        }
        if verdict.armed() {
            self.update(|status| status.arm_accepted = true);
        }
    }

    /// Refuses the arm receipt whose confirmation timeout has passed.
    pub(super) fn settle_expired_arm(&mut self, now_ns: u64) {
        if let Some((mission_id, verdict)) = self.flight.arm.expire(now_ns) {
            self.settle_arm(mission_id, verdict);
        }
    }
}
