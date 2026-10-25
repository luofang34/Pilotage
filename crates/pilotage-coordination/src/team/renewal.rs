//! An approval sent to a running team (ADR-0052): the team finds what it is
//! for with the rule a vehicle lane uses. A renewal of the team plan names
//! the team's run; the team passes it on to each member that may still fly,
//! as a renewal of that member's own plan and run, and answers once they
//! answered. A member that finished gets none. A member whose link is lost
//! or that a person stated down gets none either, and the receipt names it.
//! A member's renewal ends only the hold its approval's expiry applied.

use pilotage_mission::{Approvable, ApprovalTarget, IntentContext, admit_approval};
use pilotage_protocol::wire;
use wire::MissionLaneState as State;

use super::intents::delegate;
use super::{MemberIntent, Op, TeamLane};

/// A renewal waiting for its members' answers.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Renewal {
    /// Members that refused, with their reasons.
    refused: Vec<String>,
    /// Members the renewal was not sent to, with why.
    skipped: Vec<String>,
}

/// The member's run may still fly.
fn may_fly(status: &wire::MissionStatus) -> bool {
    matches!(
        State::try_from(status.state),
        Ok(State::Running | State::HoldRequested | State::Held | State::Uncertain)
    )
}

impl TeamLane {
    /// An approval while the team runs: a renewal of the team plan, or the
    /// approval of a drafted revision.
    pub(super) fn approve_running(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        let plan = self
            .proposal
            .as_ref()
            .map(|p| p.digest.clone())
            .unwrap_or_default();
        let drafted = self
            .revision
            .as_ref()
            .filter(|r| r.stage == wire::MissionSuccessorState::Drafted)
            .map(|r| r.proposal.digest.clone());
        let held = Approvable {
            plan: &plan,
            run: self.run_id.as_deref(),
            drafted: drafted.as_deref(),
        };
        // Nothing changes and nothing is passed on before the approval is
        // admitted.
        let (target, envelope) = admit_approval(approval, held, context.now_unix_ms)?;
        self.within_height(envelope)?;
        if target != ApprovalTarget::Renewal {
            self.drafted_within_height(envelope)?;
        }
        match target {
            ApprovalTarget::Renewal => self.renew(approval),
            ApprovalTarget::Successor | ApprovalTarget::Plan => self
                .approve_revision(approval, context)
                .map(|intents| (intents, false)),
        }
    }

    fn renew(
        &mut self,
        approval: &wire::MissionApproval,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        self.busy(Op::Renew)?;
        self.approval = Some(approval.clone());
        let mut renewal = Renewal::default();
        let mut intents = Vec::new();
        let members: Vec<String> = self.member_proposals.keys().cloned().collect();
        for member in members {
            if self.lost.contains_key(&member) {
                renewal
                    .skipped
                    .push(format!("{member}: its link is lost or it is stated down"));
                continue;
            }
            let Some(status) = self.statuses.get(&member).filter(|s| may_fly(s)) else {
                continue;
            };
            let band: Vec<wire::GeoPoint> =
                status.tasks.iter().flat_map(|t| t.path.clone()).collect();
            let launch = self.member(&member).map(|m| m.launch);
            let mut renewed = delegate(
                approval,
                &status.run_id,
                &status.proposal_digest,
                &band,
                launch,
                &self.config.name,
            );
            renewed.run_id.clone_from(&status.run_id);
            let renewed = self.with_share(&member, renewed);
            intents.push(self.request(
                &member,
                Op::Renew,
                None,
                wire::mission_intent::Action::Approve(renewed),
            ));
        }
        self.detail = format!("renewing the approval with {} members", intents.len());
        let deferred = !intents.is_empty();
        self.renewal = Some(renewal);
        if !deferred {
            self.detail = "the approval is renewed; no member flies".to_owned();
        }
        Ok((intents, deferred))
    }

    /// A member's answer to its renewal. Returns the operator's receipt
    /// once every member answered.
    pub(super) fn on_renew_receipt(
        &mut self,
        member: &str,
        accepted: bool,
        receipt: &wire::MissionReceipt,
        operation: Option<&str>,
    ) -> Option<wire::MissionReceipt> {
        // An answer to a renewal that ended changes only its member: it
        // counts in no renewal's tally.
        let renewal = self.renewal_of(operation)?;
        if !accepted {
            renewal
                .refused
                .push(format!("{member}: {}", receipt.reason));
        }
        if self.group_waits(operation, Op::Renew) {
            return None;
        }
        let (renewed, reason) = self.renewal_of(operation).map(|r| r.verdict())?;
        self.detail = if reason.is_empty() {
            "the approval is renewed".to_owned()
        } else {
            format!("the approval is renewed; {reason}")
        };
        self.finish(operation, renewed, reason, None)
    }
}

impl Renewal {
    /// Whether every member asked renewed, and what the receipt says of
    /// the members that refused or were not asked.
    pub(super) fn verdict(&self) -> (bool, String) {
        let mut reason = Vec::new();
        if !self.refused.is_empty() {
            reason.push(format!("refused by {}", self.refused.join("; ")));
        }
        if !self.skipped.is_empty() {
            reason.push(format!("not sent to {}", self.skipped.join("; ")));
        }
        (self.refused.is_empty(), reason.join("; "))
    }
}
