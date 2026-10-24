//! An approval that expires while its run flies (ADR-0052). The lane
//! applies the recovery the approval names once: it holds in place, or it
//! ends after the line in progress and flies home. A run under an expired
//! approval resumes, and a waiting revision applies, only after a new
//! approval of the plan renews it.

use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionSuccessorState as Stage;

use super::successor::Successor;
use super::{IntentContext, Lane, LaneEffect, approval, compile};

type Outcome = Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String>;

impl Lane {
    /// Takes civil time. When the active approval expired during the run,
    /// applies the recovery it names and returns the effect for the host.
    pub fn on_clock(&mut self, now_unix_ms: u64) -> Option<LaneEffect> {
        self.clock_unix_ms = self.clock_unix_ms.max(now_unix_ms);
        if self.expired
            || !matches!(
                self.state,
                State::Running | State::HoldRequested | State::Held
            )
        {
            return None;
        }
        let approval = self.approval.as_ref()?;
        if approval::still_valid(approval, self.clock_unix_ms).is_ok() {
            return None;
        }
        let policy = approval
            .envelope
            .as_ref()
            .map_or(wire::MissionExpiryPolicy::Unspecified, |e| e.on_expiry());
        self.expired = true;
        self.changed = true;
        if policy == wire::MissionExpiryPolicy::ReturnHome {
            return self.recover_home();
        }
        self.detail =
            "the approval expired; the run holds until its plan is approved again".to_owned();
        self.add_hold(wire::MissionHoldReason::ApprovalExpired);
        if self.state == State::Running {
            self.state = State::HoldRequested;
            return Some(LaneEffect::Hold);
        }
        None
    }

    /// Whether the active approval has expired by the latest known time.
    pub(super) fn approval_expired(&self) -> bool {
        self.expired
            || self
                .approval
                .as_ref()
                .is_some_and(|a| approval::still_valid(a, self.clock_unix_ms).is_err())
    }

    /// A new approval of the plan the run flies: it replaces the active one.
    pub(super) fn renew(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Outcome {
        let current = self.proposal.as_ref().ok_or("no plan to approve")?;
        approval::admit(approval, current, &self.config, context.now_unix_ms)?;
        self.approval = Some(approval.clone());
        self.expired = false;
        self.holds
            .retain(|r| *r != wire::MissionHoldReason::ApprovalExpired);
        self.detail = "the approval of the plan in progress is renewed".to_owned();
        Ok((self.resume_when_free(), None))
    }

    /// The return home an expired approval names: a successor that ends
    /// after the line in progress, admitted under the expired approval only
    /// for that recovery.
    fn recover_home(&mut self) -> Option<LaneEffect> {
        let current = self.proposal.as_ref()?;
        let run = self.run.as_ref()?;
        let flown = run.flown_lines();
        let fresh_id = format!("{}-expired", run.run_id());
        let mut compiled = compile::home(current, fresh_id);
        self.revisions = self.revisions.wrapping_add(1);
        compile::succeed(
            &mut compiled,
            &current.proposal.digest,
            "the approval expired: end after the line in progress and return to the launch point"
                .to_owned(),
        );
        let delegated = wire::MissionApproval {
            proposal_digest: compiled.proposal.digest.clone(),
            ..self.approval.clone()?
        };
        self.successor = Some(Successor {
            compiled,
            stage: Stage::Drafted,
            drafted_by: "the approval's expiry".to_owned(),
            flown_at_draft: flown,
            recovery: true,
            release: None,
        });
        self.detail =
            "the approval expired; the run returns home after the line in progress".to_owned();
        self.admit_successor(delegated)
    }
}
