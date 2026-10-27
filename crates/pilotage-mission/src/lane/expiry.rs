//! An approval that expires while its run flies (ADR-0052). The lane
//! applies the recovery the approval names once: it holds in place, or it
//! ends after the line in progress and flies home. A run under an expired
//! approval resumes, and a waiting revision applies, only after a new
//! approval of the plan renews it.

use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionSuccessorState as Stage;

use super::successor::{Basis, Successor};
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
        if super::target::still_valid(approval, self.clock_unix_ms).is_ok() {
            return None;
        }
        let policy = approval
            .envelope
            .as_ref()
            .map_or(wire::MissionExpiryPolicy::Unspecified, |e| e.on_expiry());
        self.expired = true;
        self.changed = true;
        // A plan of operations has no way home: it holds before its next
        // operation, as for a hold on expiry.
        let flies = self.proposal.as_ref().is_some_and(compile::Compiled::flies);
        self.detail =
            "the approval expired; the run holds until its plan is approved again".to_owned();
        if policy == wire::MissionExpiryPolicy::ReturnHome && flies {
            match self.recover_home() {
                Ok(effect) => return effect,
                Err(refusal) => {
                    self.detail = format!(
                        "the approval expired; the way home is refused, so the run holds until its plan is approved again: {refusal}"
                    );
                }
            }
        }
        self.add_hold(wire::MissionHoldReason::ApprovalExpired);
        if self.state == State::Running {
            self.state = State::HoldRequested;
            return Some(LaneEffect::Hold);
        }
        None
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
    fn recover_home(&mut self) -> Result<Option<LaneEffect>, String> {
        let (Some(current), Some(run), Some(approval)) = (
            self.proposal.as_ref(),
            self.run.as_ref(),
            self.approval.clone(),
        ) else {
            return Ok(None);
        };
        let flown = run.flown_lines();
        let fresh_id = format!("{}-expired", run.run_id());
        let mut compiled = compile::home(current, fresh_id);
        // The lane starts this recovery on its own, so it flies only inside
        // the envelope the expired approval gave.
        let envelope = approval
            .envelope
            .as_ref()
            .ok_or("the approval has no adaptation envelope")?;
        approval::within(&compiled, envelope)?;
        self.revisions = self.revisions.wrapping_add(1);
        compile::succeed(
            &mut compiled,
            &current.proposal.digest,
            "the approval expired: end after the line in progress and return to the launch point"
                .to_owned(),
        );
        let delegated = wire::MissionApproval {
            proposal_digest: compiled.proposal.digest.clone(),
            ..approval
        };
        self.successor = Some(Successor {
            compiled,
            stage: Stage::Drafted,
            drafted_by: "the approval's expiry".to_owned(),
            flown_at_draft: flown,
            basis: Basis::Drafted,
            approval: None,
            release: None,
            share_of: String::new(),
        });
        self.detail =
            "the approval expired; the run returns home after the line in progress".to_owned();
        Ok(self.admit_successor(delegated, Basis::ExpiryRecovery))
    }
}
