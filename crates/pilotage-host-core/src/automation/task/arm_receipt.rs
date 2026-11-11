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
//!
//! A flight controller that is not ready yet answers an arm with a
//! temporary rejection. That is not a spent retry: the arm is sent again
//! every [`ARM_RESEND_NS`] until the controller accepts it or
//! [`ARM_READY_TIMEOUT_NS`] has passed since the first refusal, which is
//! a refused receipt of its own kind ([`ArmVerdict::NotReady`]). The
//! window ends before the engine's own receipt wait
//! (`OPERATIONAL_RECEIPT_TIMEOUT_NS`, 5 s), so the engine always hears
//! the answer.
//!
//! The controller's acknowledgement that accepts the arm is the arm's
//! answer: it says the vehicle armed. Waiting for the next armed state
//! report would cost up to one heartbeat period (a second), and a run
//! that leaves the engine without a receipt that long leaves the session
//! without frames. A disarm report after it still ends the run
//! (`flight_watch`).

use pilotage_mission::MissionAction;
use pilotage_protocol::{CommandOutcome, wire};
use tracing::{info, warn};

/// How long after a temporary rejection the arm is sent again.
pub(super) const ARM_RESEND_NS: u64 = 500_000_000;

/// How long the flight controller may stay not ready for an arm, from its
/// first temporary rejection.
pub(super) const ARM_READY_TIMEOUT_NS: u64 = 4_000_000_000;

/// The flight controller's answer to an arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArmVerdict {
    /// The flight controller reports itself armed.
    Armed,
    /// The flight controller's acknowledgement refuses the arm.
    Refused(CommandOutcome),
    /// No answer came before the confirmation timeout.
    Unanswered,
    /// The flight controller stayed not ready for the arm for
    /// [`ARM_READY_TIMEOUT_NS`].
    NotReady,
}

impl ArmVerdict {
    /// Whether the verdict is an accepted receipt.
    pub(super) const fn armed(self) -> bool {
        matches!(self, Self::Armed)
    }
}

/// One arm that waits for the vehicle's answer.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct ArmReceipt {
    pending: Option<Pending>,
    /// The arm the engine asked for, to send again while the flight
    /// controller is not ready.
    pub(super) action: Option<MissionAction>,
    /// When the not-ready window of the current arm ends.
    ready_deadline_ns: Option<u64>,
    /// When the arm is sent again.
    resend_at_ns: Option<u64>,
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
        let answer = (fc.last_command_kind == 1)
            .then(|| CommandOutcome::from_wire(fc.last_command_outcome))
            .flatten();
        let armed =
            (fc.stamp.is_some() && fc.arm_state == 2) || answer == Some(CommandOutcome::Accepted);
        let refusal = answer.filter(|outcome| outcome.is_refusal());
        let verdict = if armed {
            ArmVerdict::Armed
        } else {
            ArmVerdict::Refused(refusal?)
        };
        self.pending = None;
        Some((pending.mission_id, verdict))
    }

    /// Waits for the flight controller to become ready for the arm after a
    /// temporary rejection at `now_ns`: true while the not-ready window is
    /// open, with the arm due again at `now_ns + ARM_RESEND_NS`.
    pub(super) fn wait_for_readiness(&mut self, now_ns: u64) -> bool {
        let deadline = *self
            .ready_deadline_ns
            .get_or_insert(now_ns.saturating_add(ARM_READY_TIMEOUT_NS));
        if now_ns >= deadline {
            self.ready_deadline_ns = None;
            self.resend_at_ns = None;
            return false;
        }
        self.resend_at_ns = Some(now_ns.saturating_add(ARM_RESEND_NS));
        true
    }

    /// The arm to send again, once it is due at `now_ns`.
    pub(super) fn resend_due(&mut self, now_ns: u64) -> Option<MissionAction> {
        self.resend_at_ns.filter(|at| now_ns >= *at)?;
        self.resend_at_ns = None;
        self.action
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
        let mut verdict = verdict;
        if verdict == ArmVerdict::Refused(CommandOutcome::TemporarilyRejected) {
            let now = self.mission_now().as_nanos();
            if self.flight.arm.wait_for_readiness(now) {
                info!("the flight controller is not ready for the arm; it is sent again");
                return;
            }
            warn!("the flight controller stayed not ready for the arm");
            verdict = ArmVerdict::NotReady;
        }
        self.flight.arm.ready_deadline_ns = None;
        info!(?verdict, "the flight controller answered the mission arm");
        if let Some(mission) = self.mission.as_mut() {
            mission.on_action_result(mission_id, verdict.armed());
        }
        if verdict.armed() {
            self.update(|status| status.arm_accepted = true);
        }
    }

    /// Refuses the arm receipt whose confirmation timeout has passed, and
    /// sends again an arm the flight controller was not ready for.
    pub(super) fn settle_expired_arm(&mut self, now_ns: u64) {
        if let Some((mission_id, verdict)) = self.flight.arm.expire(now_ns) {
            self.settle_arm(mission_id, verdict);
        }
        if let Some(action) = self.flight.arm.resend_due(now_ns) {
            self.send_action(action);
        }
    }
}

#[cfg(test)]
mod tests;
