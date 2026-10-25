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
    /// The approvals the lane would take now: a plan need only while it is
    /// proposed, a successor and a renewal need only while its run flies.
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
            // A team that made the objective for this member approves its plan.
            let owed = plan
                .objective
                .as_ref()
                .map(|o| o.delegated_by.clone())
                .unwrap_or_default();
            needs.push(need(
                Target::Plan,
                &plan.digest,
                String::new(),
                owed,
                "the plan waits for its approval",
            ));
        }
        if !self.is_flying() {
            return needs;
        }
        if let Some(successor) = self
            .successor
            .as_ref()
            .filter(|s| s.stage == Stage::Drafted)
        {
            // The team that granted the approval in force approves its own share.
            let owed = if !successor.share_of.is_empty() && successor.share_of == delegated_by {
                successor.share_of.clone()
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
            .hold_reasons()
            .contains(&(wire::MissionHoldReason::ApprovalExpired as i32))
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
