//! Why a run holds (ADR-0052). Each reason has its own way out: an
//! operator's resume removes the operator's hold, and a renewal of the
//! approval removes the hold its expiry applied. The run resumes only when
//! no reason stays, so a renewal never ends an operator's hold, and a
//! resume never runs on an expired approval.

use pilotage_protocol::wire;
use wire::MissionHoldReason as Reason;
use wire::MissionLaneState as State;

use super::{IntentContext, Lane, LaneEffect, approval};

type Outcome = Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String>;

impl Lane {
    /// An operator's hold: a running run is asked to hold, and a held run
    /// keeps holding for the operator too.
    pub(super) fn hold(&mut self) -> Outcome {
        match self.state {
            State::Running => {
                self.add_hold(Reason::Operator);
                self.state = State::HoldRequested;
                self.detail = "holdrequested".to_owned();
                Ok((Some(LaneEffect::Hold), None))
            }
            State::HoldRequested | State::Held => {
                self.add_hold(Reason::Operator);
                Ok((None, None))
            }
            _ => Err(format!("not possible while the lane is {:?}", self.state)),
        }
    }

    /// An operator's resume, under an approval still valid at `context`'s time.
    pub(super) fn resume(&mut self, context: &IntentContext) -> Outcome {
        if !matches!(self.state, State::Held | State::HoldRequested) {
            return Err(format!("not possible while the lane is {:?}", self.state));
        }
        let approval = self
            .approval
            .as_ref()
            .ok_or("no approval to resume under")?;
        if self.holds.contains(&Reason::ApprovalExpired) {
            return Err("the approval expired; approve the plan again to resume".to_owned());
        }
        approval::still_valid(approval, context.now_unix_ms)
            .map_err(|reason| format!("{reason}; approve the plan again to resume"))?;
        self.holds.retain(|r| *r != Reason::Operator);
        Ok((self.resume_when_free(), None))
    }

    pub(super) fn add_hold(&mut self, reason: Reason) {
        if !self.holds.contains(&reason) {
            self.holds.push(reason);
        }
    }

    /// Resumes a held run that has no reason left to hold.
    pub(super) fn resume_when_free(&mut self) -> Option<LaneEffect> {
        if !self.holds.is_empty() || !matches!(self.state, State::Held | State::HoldRequested) {
            return None;
        }
        self.state = State::Running;
        self.detail = "running".to_owned();
        Some(LaneEffect::Resume)
    }

    /// The reasons the status reports: none unless the run holds.
    pub(super) fn hold_reasons(&self) -> Vec<i32> {
        if matches!(self.state, State::Held | State::HoldRequested) {
            self.holds.iter().map(|r| *r as i32).collect()
        } else {
            Vec::new()
        }
    }
}
