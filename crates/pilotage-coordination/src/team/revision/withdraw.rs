//! A revision that no longer applies (ADR-0052): the giving member of a
//! reassignment refused its share, ended its run without the release, or
//! is stated down. Its approval gives way to the one it replaced.

use pilotage_protocol::wire;
use wire::MissionSuccessorState as Stage;

use super::super::TeamLane;

impl TeamLane {
    /// A member's share of the revision is refused, and `member` keeps its
    /// work. A reassignment has no other share, so it no longer applies;
    /// the other members of a kept-lines revision still apply theirs.
    pub(super) fn refused_share(&mut self, member: &str, reason: &str) {
        let basis = format!("{member} refused: {reason}; it keeps the work");
        if let Some((digest, replaced)) = self.withdraw_handover(member, basis) {
            self.withdraw_revision(&digest, replaced);
        }
    }

    /// `member` is stated down: it never released the work of its open
    /// handover and never will, so the reassignment no longer applies.
    pub(in crate::team) fn share_never_comes(&mut self, member: &str) {
        let basis = format!("{member} is down and released nothing; a person may move its work");
        self.release_never_comes(member, basis);
    }

    /// `member`'s run ended, as `status` reports, without the release of
    /// its open handover: the release can no longer come.
    pub(in crate::team) fn release_ended(&mut self, member: &str, status: &wire::MissionStatus) {
        use wire::MissionLaneState as State;
        if matches!(
            State::try_from(status.state),
            Ok(State::Complete | State::Aborted | State::Failed)
        ) {
            let basis = format!("{member}'s run ended before it released the work; it keeps it");
            self.release_never_comes(member, basis);
        }
    }

    fn release_never_comes(&mut self, member: &str, basis: String) {
        if let Some((digest, replaced)) = self.withdraw_handover(member, basis) {
            self.withdraw_revision(&digest, replaced);
        }
    }

    /// The revision of `digest` no longer applies. While it is the team's
    /// revision, it is withdrawn; while its approval is in force, the
    /// approval it `replaced` is given back. A later revision or a renewal
    /// stays as it is.
    fn withdraw_revision(&mut self, digest: &[u8], replaced: Option<wire::MissionApproval>) {
        if let Some(revision) = self
            .revision
            .as_mut()
            .filter(|r| r.proposal.digest == digest)
        {
            revision.stage = Stage::Withdrawn;
        }
        let brought = self
            .approval
            .as_ref()
            .is_some_and(|a| a.proposal_digest == digest);
        // A later revision that replaced this one's approval gives back
        // the approval this one replaced, never this one's.
        let named = |a: &Option<wire::MissionApproval>| {
            a.as_ref().is_some_and(|a| a.proposal_digest == digest)
        };
        for handover in &mut self.handovers {
            if named(&handover.replaced_approval) {
                handover.replaced_approval.clone_from(&replaced);
            }
        }
        if let Some(revision) = self.revision.as_mut()
            && named(&revision.replaced_approval)
        {
            revision.replaced_approval.clone_from(&replaced);
        }
        if let Some(replaced) = replaced
            && brought
        {
            self.approval = Some(replaced);
        }
    }
}
