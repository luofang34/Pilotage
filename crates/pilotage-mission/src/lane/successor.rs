//! Planned changes to a plan in progress (ADR-0052): a revision compiles
//! into a successor that the receipt and the status show for review. An
//! approval of its digest admits it, or the lane admits it at once when
//! the sender asked and the change stays inside the active envelope. A
//! running lane switches at its next checkpoint: the end of the line in
//! progress, or at once while held or flying between lines. A line in
//! progress is never cut short; a transit to the next line is, so a line
//! the revision gives away is not flown on the way.

use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionSuccessorState as Stage;

use super::{IntentContext, Lane, LaneEffect, RunOrder, approval, compile};
use compile::Compiled;

/// A revision of the plan in progress.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Successor {
    pub compiled: Compiled,
    pub stage: Stage,
    pub drafted_by: String,
    /// Lines flown when it was drafted; one more is the checkpoint.
    pub flown_at_draft: usize,
    /// The recovery an expired approval names, which applies under it.
    pub recovery: bool,
}

type Outcome = Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String>;

fn short(digest: &[u8]) -> String {
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

impl Lane {
    pub(super) fn revise(
        &mut self,
        revise: &wire::MissionRevise,
        context: &IntentContext,
    ) -> Outcome {
        use wire::mission_revise::Change;
        let change = revise
            .change
            .as_ref()
            .ok_or("the revision names no change")?;
        let current = self.proposal.as_ref().ok_or("there is no plan to revise")?;
        if revise.base_digest != current.proposal.digest {
            return Err(format!(
                "the revision continues plan {}; the plan is now {}",
                short(&revise.base_digest),
                short(&current.proposal.digest)
            ));
        }
        if matches!(self.state, State::Proposed | State::Admitted) {
            // Before a start, a revised objective is simply the new draft.
            let Change::Objective(objective) = change else {
                return Err("before a start, only a revised objective replaces the plan".to_owned());
            };
            return self.propose(objective, context);
        }
        if !matches!(
            self.state,
            State::Running | State::HoldRequested | State::Held
        ) {
            return Err(format!("nothing to revise; the lane is {:?}", self.state));
        }
        let run = self.run.as_ref().ok_or("no run to revise")?;
        if revise.run_id != run.run_id() {
            return Err(format!(
                "the revision continues run {}; the run is now {}",
                revise.run_id,
                run.run_id()
            ));
        }
        let (mut compiled, summary) = self.compile_change(change, current, context)?;
        self.revisions = self.revisions.wrapping_add(1);
        compile::succeed(
            &mut compiled,
            self.revisions,
            &current.proposal.digest,
            summary,
        );
        let proposal = compiled.proposal.clone();
        self.successor = Some(Successor {
            compiled,
            stage: Stage::Drafted,
            drafted_by: revise
                .drafted_by
                .as_ref()
                .map(|origin| origin.subject.clone())
                .unwrap_or_default(),
            flown_at_draft: run.flown_lines(),
            recovery: false,
        });
        self.detail = "revision drafted; waiting for review".to_owned();
        if !revise.within_envelope {
            return Ok((None, Some(proposal)));
        }
        // Outside the envelope the draft stays, for an approval to admit.
        match self.admit_within_envelope(context) {
            Ok(effect) => Ok((effect, Some(proposal))),
            Err(reason) => {
                self.detail = reason;
                Ok((None, Some(proposal)))
            }
        }
    }

    /// The plan a change asks for, and what it changes in words.
    fn compile_change(
        &self,
        change: &wire::mission_revise::Change,
        current: &Compiled,
        context: &IntentContext,
    ) -> Result<(Compiled, String), String> {
        use wire::mission_revise::Change;
        match change {
            Change::Objective(objective) => {
                let compiled = compile::compile(objective, &self.config, context.fresh_id.clone())?;
                let lines = compiled.proposal.estimates.map_or(0, |e| e.lines);
                let title = &objective.title;
                Ok((
                    compiled,
                    format!("fly the revised objective \"{title}\": {lines} lines"),
                ))
            }
            Change::Finish(_) => Ok((
                compile::home(current, context.fresh_id.clone()),
                "end after the line in progress and return to the launch point".to_owned(),
            )),
            Change::Reassign(_) => Err(
                "a reassignment moves work between a team's members; this lane is one vehicle's"
                    .to_owned(),
            ),
        }
    }

    /// Admits the drafted successor under the active approval, delegated
    /// to its digest, when the envelope covers it.
    fn admit_within_envelope(
        &mut self,
        context: &IntentContext,
    ) -> Result<Option<LaneEffect>, String> {
        let active = self
            .approval
            .clone()
            .ok_or("no approval to revise inside")?;
        let envelope = active
            .envelope
            .clone()
            .ok_or("the approval has no envelope")?;
        if !envelope.permit_revise {
            return Err("the approval does not permit revisions inside its envelope; approve the draft instead".to_owned());
        }
        let successor = self.successor.as_ref().ok_or("no drafted revision")?;
        let delegated = wire::MissionApproval {
            proposal_digest: successor.compiled.proposal.digest.clone(),
            ..active
        };
        approval::admit(
            &delegated,
            &successor.compiled,
            &self.config,
            context.now_unix_ms,
        )
        .map_err(|reason| {
            format!("outside the envelope ({reason}); the draft waits for an approval")
        })?;
        Ok(self.admit_successor(delegated))
    }

    /// An approval of the drafted successor's digest.
    pub(super) fn approve_successor(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Outcome {
        let successor = self
            .successor
            .as_ref()
            .filter(|s| {
                s.stage == Stage::Drafted && s.compiled.proposal.digest == approval.proposal_digest
            })
            .ok_or("no drafted revision has that digest")?;
        approval::admit(
            approval,
            &successor.compiled,
            &self.config,
            context.now_unix_ms,
        )?;
        self.expired = false;
        Ok((self.admit_successor(approval.clone()), None))
    }

    pub(super) fn admit_successor(
        &mut self,
        approval: wire::MissionApproval,
    ) -> Option<LaneEffect> {
        self.approval = Some(approval);
        if let Some(successor) = self.successor.as_mut() {
            successor.stage = Stage::Admitted;
        }
        let on_line = self
            .run
            .as_ref()
            .is_some_and(super::run::RunTracker::on_line);
        if matches!(self.state, State::Held | State::HoldRequested) || !on_line {
            return self.switch();
        }
        self.detail = "revision admitted; it applies at the end of the line in progress".to_owned();
        None
    }

    /// The switch when the run reached the admitted successor's
    /// checkpoint: a line ended, the way home began, or the plan ended.
    pub(super) fn at_checkpoint(&mut self, plan_complete: bool) -> Option<LaneEffect> {
        let successor = self
            .successor
            .as_ref()
            .filter(|s| s.stage == Stage::Admitted)?;
        // A revision does not apply under an expired approval; the
        // expiry's own recovery does.
        if !successor.recovery && self.approval_expired() {
            return None;
        }
        let run = self.run.as_ref()?;
        let reached = plan_complete
            || run.flying_home()
            || !run.on_line()
            || run.flown_lines() > successor.flown_at_draft;
        if reached { self.switch() } else { None }
    }

    fn switch(&mut self) -> Option<LaneEffect> {
        let successor = self.successor.as_mut()?;
        successor.stage = Stage::Applied;
        let compiled = successor.compiled.clone();
        let previous = self.run.take();
        let run_id = format!(
            "{}-rev{}",
            previous
                .as_ref()
                .map(|run| run.run_id())
                .unwrap_or_default(),
            self.revisions
        );
        if let Some(run) = previous {
            self.history.extend(run.superseded_statuses());
        }
        self.run = Some(super::run::RunTracker::new(
            run_id.clone(),
            &compiled,
            self.config.judged,
        ));
        self.state = State::Running;
        self.detail = format!(
            "flying revision {}: {}",
            self.revisions, compiled.proposal.summary
        );
        let order = RunOrder {
            run_id,
            plan: compiled.plan.clone(),
            config: compiled.config.clone(),
        };
        self.proposal = Some(compiled);
        self.changed = true;
        Some(LaneEffect::Start(Box::new(order)))
    }

    /// The successor as the status reports it.
    pub(super) fn successor_status(&self) -> Option<wire::MissionSuccessor> {
        let successor = self.successor.as_ref()?;
        let applies_after = match successor.stage {
            Stage::Admitted if matches!(self.state, State::Held | State::HoldRequested) => {
                "now: the run is held".to_owned()
            }
            Stage::Drafted | Stage::Admitted => self
                .run
                .as_ref()
                .and_then(super::run::RunTracker::active_task)
                .unwrap_or_default(),
            _ => String::new(),
        };
        Some(wire::MissionSuccessor {
            digest: successor.compiled.proposal.digest.clone(),
            summary: successor.compiled.proposal.summary.clone(),
            state: successor.stage as i32,
            applies_after,
            drafted_by: successor.drafted_by.clone(),
        })
    }
}
