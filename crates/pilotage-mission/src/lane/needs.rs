//! The approvals a lane needs now (ADR-0052), for clients to raise: its
//! plan before the start, a drafted successor, and the renewal of an
//! expired approval. When a team passed the approval in force down to this
//! lane, the team owes its own share of a revision and the renewal of what
//! it granted, and the need names it; a client raises a need for a person
//! only when no lane owes it.

use pilotage_protocol::wire;
use wire::MissionApprovalTarget as Target;
use wire::MissionLaneState as State;
use wire::MissionSuccessorState as Stage;

use super::Lane;

impl Lane {
    pub(super) fn approval_needs(&self) -> Vec<wire::MissionApprovalNeed> {
        let mut needs = Vec::new();
        let Some(plan) = self.proposal.as_ref().map(|c| &c.proposal) else {
            return needs;
        };
        let run_id = self
            .run
            .as_ref()
            .map(super::run::RunTracker::run_id)
            .unwrap_or_default();
        let delegated_by = self
            .approval
            .as_ref()
            .map(|a| a.delegated_by.clone())
            .unwrap_or_default();
        if self.state == State::Proposed {
            needs.push(need(
                Target::Plan,
                &plan.digest,
                String::new(),
                String::new(),
                "the plan waits for its approval",
            ));
        }
        if let Some(successor) = self
            .successor
            .as_ref()
            .filter(|s| s.stage == Stage::Drafted)
        {
            let granted_by = self
                .approval
                .as_ref()
                .and_then(|a| a.approver.as_ref())
                .map(|o| o.subject.as_str());
            // The principal that granted the approval in force drafted it: a
            // team's own share of its revision, which the team approves.
            let owed = if granted_by == Some(successor.drafted_by.as_str()) {
                delegated_by.clone()
            } else {
                String::new()
            };
            needs.push(need(
                Target::Successor,
                &successor.compiled.proposal.digest,
                run_id.clone(),
                owed,
                "a revision waits for its approval",
            ));
        }
        if self
            .holds
            .contains(&wire::MissionHoldReason::ApprovalExpired)
        {
            needs.push(need(
                Target::Renewal,
                &plan.digest,
                run_id,
                delegated_by,
                "the approval expired; the run holds until it is renewed",
            ));
        }
        needs
    }
}

/// One approval need.
pub(crate) fn need(
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
