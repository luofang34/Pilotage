//! What a run does on its own after it lost control (ADR-0052): a
//! principal took the motion lease from it, and that principal's lease
//! ended without a hand-back, so no one commands the vehicle. Under a valid
//! approval that names the way home, the lane flies home at once, as a
//! recovery that never runs the work again. Only a hand-back resumes the
//! work.

use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionSuccessorState as Stage;

use super::successor::{Basis, Successor};
use super::{Lane, LaneEffect, RunExecution, compile};

impl Lane {
    /// Whether the run lost control to a principal whose lease then ended
    /// without a hand-back.
    pub(super) fn control_freed(&self) -> bool {
        matches!(
            self.execution,
            Some(RunExecution::ControlLost { freed: true, .. })
        )
    }

    /// Whether the run lost a lease it held: no one flies its plan to a
    /// checkpoint.
    pub(super) fn control_lost(&self) -> bool {
        matches!(self.execution, Some(RunExecution::ControlLost { .. }))
    }

    /// Applies the approval's recovery for a lost control: the way home,
    /// when the approval is still valid and names it, and the plan flies.
    /// A recovery that loses control in its turn starts again from where the
    /// vehicle is. An expired approval leaves the recovery to its own policy.
    pub(super) fn recover_from_lost_control(&mut self) -> Option<LaneEffect> {
        // An expired approval fails the check below: its own policy decides.
        if !self.control_freed()
            || !matches!(
                self.state,
                State::Running | State::HoldRequested | State::Held
            )
        {
            return None;
        }
        let approval = self.approval.as_ref()?;
        super::target::still_valid(approval, self.clock_unix_ms).ok()?;
        let policy = approval
            .envelope
            .as_ref()
            .map_or(wire::MissionControlLossPolicy::Unspecified, |e| {
                e.on_control_lost()
            });
        if policy != wire::MissionControlLossPolicy::ReturnHome {
            return None;
        }
        let current = self.proposal.as_ref().filter(|c| c.flies())?;
        let run = self.run.as_ref()?;
        let flown = run.flown_lines();
        let mut compiled = compile::home(current, format!("{}-recovered", run.run_id()));
        self.revisions = self.revisions.wrapping_add(1);
        compile::succeed(
            &mut compiled,
            &current.proposal.digest,
            "control was lost and not handed back: return to the launch point".to_owned(),
        );
        let delegated = wire::MissionApproval {
            proposal_digest: compiled.proposal.digest.clone(),
            ..approval.clone()
        };
        self.successor = Some(Successor {
            compiled,
            stage: Stage::Admitted,
            drafted_by: "the approval's recovery for a lost control".to_owned(),
            flown_at_draft: flown,
            basis: Basis::ControlLossRecovery,
            approval: Some(delegated),
            release: None,
            share_of: String::new(),
        });
        self.detail =
            "control was lost and not handed back; the run returns to the launch point".to_owned();
        self.switch()
    }
}
