//! The approvals a team lane needs now, by the rule a vehicle lane uses: its
//! plan before the start, a drafted revision, and the renewal of an expired
//! approval. A parent team that passed the approval in force down owes the
//! team's share of its own revision and the renewal of what it granted.

use pilotage_protocol::wire;
use wire::MissionApprovalTarget as Target;

use super::{Phase, TeamLane};

impl TeamLane {
    pub(super) fn approval_needs(&self, holds: &[i32]) -> Vec<wire::MissionApprovalNeed> {
        let mut needs = Vec::new();
        let Some(plan) = self.proposal.as_ref() else {
            return needs;
        };
        let run_id = self.run_id.clone().unwrap_or_default();
        let delegated_by = self
            .approval
            .as_ref()
            .map(|a| a.delegated_by.clone())
            .unwrap_or_default();
        if self.phase == Phase::Proposed {
            // A parent team that made the objective for this team approves its plan.
            let owed = self
                .objective
                .as_ref()
                .map(|o| o.delegated_by.clone())
                .unwrap_or_default();
            needs.push(need(
                Target::Plan,
                &plan.digest,
                String::new(),
                owed,
                "the team plan waits for its approval",
            ));
        }
        if let Some(revision) = self
            .revision
            .as_ref()
            .filter(|r| r.stage == wire::MissionSuccessorState::Drafted)
            .filter(|_| matches!(self.phase, Phase::Running | Phase::Held | Phase::Settling))
        {
            // The parent team that granted the approval in force approves its own share.
            let owed = if !revision.share_of.is_empty() && revision.share_of == delegated_by {
                revision.share_of.clone()
            } else {
                String::new()
            };
            needs.push(need(
                Target::Successor,
                &revision.proposal.digest,
                run_id.clone(),
                owed,
                "a team revision waits for its approval",
            ));
        }
        if holds.contains(&(wire::MissionHoldReason::ApprovalExpired as i32)) {
            needs.push(need(
                Target::Renewal,
                &plan.digest,
                run_id,
                delegated_by,
                "the approval expired; members hold until it is renewed",
            ));
        }
        needs
    }
}

fn need(
    target: Target,
    digest: &[u8],
    run_id: String,
    owed_by_lane: String,
    reason: &str,
) -> wire::MissionApprovalNeed {
    wire::MissionApprovalNeed {
        target: target as i32,
        digest: digest.to_vec(),
        run_id,
        owed_by_lane,
        reason: reason.to_owned(),
    }
}
