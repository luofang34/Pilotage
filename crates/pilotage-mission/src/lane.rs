//! The mission lane of one vehicle (ADR-0052): the one place that knows
//! which objective, proposal, approval and run are current. Sans-IO: the
//! host feeds intents, run progress, captured pictures and judgments, and
//! carries out the effects the lane returns; the lane never reads a clock.
//!
//! A proposal grants nothing. A run starts only under an approval bound to
//! the proposal's digest whose adaptation envelope covers the plan.

mod approval;
mod compile;
mod digest;
mod expiry;
mod run;
mod steering;
mod successor;

#[cfg(test)]
mod tests;

use pilotage_protocol::wire;

use crate::config::MissionConfig;
use crate::survey::SurveyRequest;

pub use compile::SurveyDefaults;
pub use digest::proposal_digest;
pub use run::{FrameAssessment, RunProgress};
pub use steering::{HOST_ISSUER, Requester, Steering};

use compile::Compiled;
use run::RunTracker;

/// What a lane is configured with for every proposal it compiles.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneConfig {
    /// The lane's name, as the status names it: the vehicle's name.
    pub name: String,
    /// The vehicle is a simulation, not a physical aircraft.
    pub simulated: bool,
    /// How a plan reaches the vehicle, such as `host-guided-velocity`.
    pub execution_strategy: String,
    /// The mission config every plan starts from; compilation sets its
    /// anchor, height and speed from the objective.
    pub template: MissionConfig,
    /// The values an objective's zero fields take.
    pub defaults: SurveyDefaults,
    /// Usable battery energy, watt-hours; `None` when the vehicle states
    /// no energy model, and admission then cannot check a reserve.
    pub usable_energy_wh: Option<f64>,
    /// Mean electrical power in cruise, watts, for the energy estimate.
    pub cruise_power_w: Option<f64>,
    /// The recovery the lane applies when it cannot continue.
    pub recovery_policy: String,
}

/// What the host supplies with an intent: the civil time and a fresh,
/// globally unique identifier for any record the intent creates.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentContext {
    /// Civil time, Unix milliseconds.
    pub now_unix_ms: u64,
    /// A fresh identifier, such as a time-ordered UUID.
    pub fresh_id: String,
}

/// What a run flies.
#[derive(Debug, Clone, PartialEq)]
pub enum RunPlan {
    /// The lines of an area survey, then home when the request says so.
    Survey(SurveyRequest),
    /// Home from wherever the vehicle is: a successor that ends the work.
    Home,
}

/// A run the host must start: the plan and the config its mission engine
/// is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOrder {
    /// The run's identifier.
    pub run_id: String,
    /// What the run flies.
    pub plan: RunPlan,
    /// The mission config.
    pub config: MissionConfig,
}

/// What the host does after an intent.
#[derive(Debug, Clone, PartialEq)]
pub enum LaneEffect {
    /// Start the run.
    Start(Box<RunOrder>),
    /// Hold the run: the vehicle stops in place.
    Hold,
    /// Resume a held run.
    Resume,
    /// End the run: the vehicle stops in place and the lane releases it.
    Abort,
}

/// The receipt for an intent and the effect, when there is one.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneReply {
    /// The receipt for the sender.
    pub receipt: wire::MissionReceipt,
    /// The effect the host carries out.
    pub effect: Option<LaneEffect>,
}

/// One vehicle's mission lane.
#[derive(Debug)]
pub struct Lane {
    config: LaneConfig,
    state: wire::MissionLaneState,
    proposal: Option<Compiled>,
    approval: Option<wire::MissionApproval>,
    run: Option<RunTracker>,
    /// A revision of the plan in progress.
    successor: Option<successor::Successor>,
    /// Revisions drafted so far; numbers each successor's tasks.
    revisions: u32,
    /// The tasks of runs a successor replaced, as they ended.
    history: Vec<wire::MissionTaskStatus>,
    detail: String,
    sequence: u64,
    changed: bool,
    /// The latest civil time the lane knows, from intents and the clock.
    clock_unix_ms: u64,
    /// The active approval expired during the run and its recovery applied.
    expired: bool,
}

impl Lane {
    /// An idle lane.
    #[must_use]
    pub fn new(config: LaneConfig) -> Self {
        Self {
            config,
            state: wire::MissionLaneState::Idle,
            proposal: None,
            approval: None,
            run: None,
            successor: None,
            revisions: 0,
            history: Vec::new(),
            detail: "waiting for an objective".to_owned(),
            sequence: 0,
            changed: true,
            clock_unix_ms: 0,
            expired: false,
        }
    }

    /// The lane's state.
    #[must_use]
    pub fn state(&self) -> wire::MissionLaneState {
        self.state
    }

    /// Answers one intent.
    pub fn on_intent(
        &mut self,
        intent: &wire::MissionIntent,
        context: &IntentContext,
    ) -> LaneReply {
        use wire::mission_intent::Action;
        self.clock_unix_ms = self.clock_unix_ms.max(context.now_unix_ms);
        let result = match intent.action.as_ref() {
            Some(Action::Propose(objective)) => self.propose(objective, context),
            Some(Action::Approve(approval)) => self.approve(approval, context),
            Some(Action::Start(start)) => self.start(start, context),
            Some(Action::Hold(_)) => self.transition(
                &[wire::MissionLaneState::Running],
                wire::MissionLaneState::HoldRequested,
                LaneEffect::Hold,
            ),
            Some(Action::Resume(_)) => self.resume(context),
            Some(Action::Abort(_)) => self.abort(),
            Some(Action::Revise(revise)) => self.revise(revise, context),
            Some(Action::MemberDown(_)) => {
                Err("a vehicle lane has no members; send it to the team".to_owned())
            }
            Some(Action::Skip(_)) => Err(
                "skipping a task needs a successor plan, which this lane does not compile yet"
                    .to_owned(),
            ),
            None => Err("the intent names no action".to_owned()),
        };
        self.changed = true;
        let (outcome, reason, effect, proposal) = match result {
            Ok((effect, proposal)) => (
                wire::MissionReceiptOutcome::Accepted,
                String::new(),
                effect,
                proposal,
            ),
            Err(reason) => (wire::MissionReceiptOutcome::Refused, reason, None, None),
        };
        LaneReply {
            receipt: wire::MissionReceipt {
                request_id: intent.request_id.clone(),
                outcome: outcome as i32,
                reason,
                proposal,
                lane: self.config.name.clone(),
            },
            effect,
        }
    }

    fn propose(
        &mut self,
        objective: &wire::MissionObjective,
        context: &IntentContext,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String> {
        if self.is_flying() {
            return Err("a run is in progress; revise it instead".to_owned());
        }
        let compiled = compile::compile(objective, &self.config, context.fresh_id.clone())?;
        let proposal = compiled.proposal.clone();
        self.proposal = Some(compiled);
        self.approval = None;
        self.run = None;
        self.successor = None;
        self.revisions = 0;
        self.history.clear();
        self.state = wire::MissionLaneState::Proposed;
        self.detail = "proposed; waiting for an approval".to_owned();
        Ok((None, Some(proposal)))
    }

    fn approve(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String> {
        if self.is_flying() {
            let current = self.proposal.as_ref().map(|c| &c.proposal.digest);
            if current == Some(&approval.proposal_digest) {
                return self.renew(approval, context);
            }
            return self.approve_successor(approval, context);
        }
        if self.state != wire::MissionLaneState::Proposed {
            return Err(format!(
                "an approval needs a proposal; the lane is {:?}",
                self.state
            ));
        }
        let compiled = self.proposal.as_ref().ok_or("no proposal to approve")?;
        approval::admit(approval, compiled, &self.config, context.now_unix_ms)?;
        self.approval = Some(approval.clone());
        self.state = wire::MissionLaneState::Admitted;
        self.detail = "approved; waiting for start".to_owned();
        Ok((None, None))
    }

    fn start(
        &mut self,
        start: &wire::MissionStart,
        context: &IntentContext,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String> {
        if self.state != wire::MissionLaneState::Admitted {
            return Err(format!(
                "a start needs an approved proposal; the lane is {:?}",
                self.state
            ));
        }
        let compiled = self.proposal.as_ref().ok_or("no proposal to start")?;
        let approval = self.approval.as_ref().ok_or("no approval to start under")?;
        if start.proposal_digest != compiled.proposal.digest {
            return Err("the start names another proposal than the approved one".to_owned());
        }
        approval::still_valid(approval, context.now_unix_ms)?;
        self.expired = false;
        let run_id = context.fresh_id.clone();
        self.run = Some(RunTracker::new(run_id.clone(), compiled));
        self.state = wire::MissionLaneState::Running;
        self.detail = "running".to_owned();
        Ok((
            Some(LaneEffect::Start(Box::new(RunOrder {
                run_id,
                plan: compiled.plan.clone(),
                config: compiled.config.clone(),
            }))),
            None,
        ))
    }

    fn abort(&mut self) -> Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String> {
        if !self.is_flying() {
            return Err(format!("nothing to abort; the lane is {:?}", self.state));
        }
        self.state = wire::MissionLaneState::Aborted;
        self.successor = None;
        self.detail = "aborted by intent; the vehicle stops in place".to_owned();
        if let Some(run) = self.run.as_mut() {
            run.close_aborted();
        }
        Ok((Some(LaneEffect::Abort), None))
    }

    fn transition(
        &mut self,
        from: &[wire::MissionLaneState],
        to: wire::MissionLaneState,
        effect: LaneEffect,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposal>), String> {
        if !from.contains(&self.state) {
            return Err(format!("not possible while the lane is {:?}", self.state));
        }
        self.state = to;
        self.detail = format!("{to:?}").to_lowercase();
        Ok((Some(effect), None))
    }

    fn is_flying(&self) -> bool {
        matches!(
            self.state,
            wire::MissionLaneState::Running
                | wire::MissionLaneState::HoldRequested
                | wire::MissionLaneState::Held
                | wire::MissionLaneState::Uncertain
        )
    }

    /// Folds the run's progress in. A plan that completes ends the run,
    /// unless an admitted successor takes over: then the host starts the
    /// successor's run, the effect returned.
    pub fn on_progress(&mut self, progress: &RunProgress) -> Option<LaneEffect> {
        let flying = self.is_flying();
        let run = self.run.as_mut()?;
        if run.on_progress(progress) {
            self.changed = true;
        }
        if self.state == wire::MissionLaneState::HoldRequested && progress.holding {
            self.state = wire::MissionLaneState::Held;
            self.detail = "held in place".to_owned();
            self.changed = true;
        }
        if flying && let Some(effect) = self.at_checkpoint(progress.plan_complete) {
            return Some(effect);
        }
        let run = self.run.as_mut()?;
        if progress.plan_complete && flying {
            run.close();
            self.state = wire::MissionLaneState::Complete;
            self.detail = run.summary();
            self.changed = true;
        } else if progress.ended && flying {
            run.close();
            self.state = wire::MissionLaneState::Failed;
            self.detail = progress
                .ended_because
                .unwrap_or("the run ended before its plan completed")
                .to_owned();
            self.changed = true;
        }
        None
    }

    /// Counts a captured picture against the active survey line. Returns
    /// the line's task identifier when one is active.
    pub fn on_frame(&mut self) -> Option<String> {
        if !self.is_flying() {
            return None;
        }
        let task = self.run.as_mut()?.on_frame()?;
        self.changed = true;
        Some(task)
    }

    /// Records a judgment of a picture captured on `task_id`.
    pub fn on_assessment(&mut self, task_id: &str, assessment: FrameAssessment) {
        if let Some(run) = self.run.as_mut()
            && run.on_assessment(task_id, assessment)
        {
            self.changed = true;
        }
    }

    /// The status as it stands, with the sequence of the last change.
    #[must_use]
    pub fn status(&self) -> wire::MissionStatus {
        let proposal = self.proposal.as_ref().map(|compiled| &compiled.proposal);
        let survey = proposal
            .and_then(|p| p.objective.as_ref())
            .and_then(|o| o.kind.as_ref())
            .map(|wire::mission_objective::Kind::Survey(survey)| survey);
        wire::MissionStatus {
            lane: self.config.name.clone(),
            state: self.state as i32,
            objective_id: proposal.map(|p| p.objective_id.clone()).unwrap_or_default(),
            proposal_digest: proposal.map(|p| p.digest.clone()).unwrap_or_default(),
            plan_digest: proposal
                .and_then(|p| p.assumptions.as_ref())
                .map(|a| a.plan_digest.clone())
                .unwrap_or_default(),
            run_id: self
                .run
                .as_ref()
                .map(RunTracker::run_id)
                .unwrap_or_default(),
            tasks: self
                .history
                .iter()
                .cloned()
                .chain(match (self.run.as_ref(), proposal) {
                    (Some(run), _) => run.task_statuses(),
                    (None, Some(p)) => run::planned_statuses(&p.tasks),
                    (None, None) => Vec::new(),
                })
                .collect(),
            active_task: self
                .run
                .as_ref()
                .and_then(RunTracker::active_task)
                .unwrap_or_default(),
            recovery_policy: self.config.recovery_policy.clone(),
            estimates: proposal.and_then(|p| p.estimates),
            simulated: self.config.simulated,
            detail: self.detail.clone(),
            members: Vec::new(),
            sequence: self.sequence,
            area: survey.map(|s| s.area.clone()).unwrap_or_default(),
            height_m: survey.map_or(0.0, |s| s.height_m),
            successor: self.successor_status(),
        }
    }

    /// The status when it changed since the last call, with a new
    /// sequence number.
    pub fn take_changed(&mut self) -> Option<wire::MissionStatus> {
        if !self.changed {
            return None;
        }
        self.changed = false;
        self.sequence = self.sequence.wrapping_add(1);
        Some(self.status())
    }
}
