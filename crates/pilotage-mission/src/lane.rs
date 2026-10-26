//! The mission lane of one vehicle (ADR-0052): the one place that knows
//! which objective, proposal, approval and run are current. Sans-IO: the
//! host feeds intents, run progress, captured pictures and judgments, and
//! carries out the effects the lane returns; the lane never reads a clock.
//!
//! A proposal grants nothing. A run starts only under an approval bound to
//! the proposal's digest whose adaptation envelope covers the plan.

mod answer;
mod approval;
mod compile;
mod evidence;
mod expiry;
mod hold;
mod needs;
mod recovery;
mod restore;
mod run;
mod settle;
mod status;
mod steering;
mod successor;
mod target;

#[cfg(test)]
mod tests;

use pilotage_protocol::wire;

use crate::config::MissionConfig;
use crate::survey::SurveyRequest;

pub use compile::SurveyDefaults;
pub use restore::{Reconciled, StatusQuery, operation_id};
pub use run::{FrameAssessment, RunExecution, RunProgress};
pub use steering::{HOST_ISSUER, Requester, Steering};
pub use target::{Approvable, ApprovalTarget, admit_approval, approval_target, valid_approval};

use compile::Compiled;
pub use evidence::{FrameTicket, VerdictRefused};
use run::RunTracker;

/// Replaced runs that may wait for answers at one time.
const MAX_SETTLING_RUNS: usize = 4;

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
    /// A judgment service judges the run's pictures, so a survey line is
    /// done only with enough pictures judged usable.
    pub judged: bool,
    /// How long a task whose execution ended still takes the answers to its
    /// pictures, milliseconds on the host's monotonic clock. The clock does
    /// not stop while the run holds.
    pub evidence_window_ms: u64,
    /// The operations the lane's host carries out: a plan with another one,
    /// or another version of one, is refused when it is proposed.
    pub executors: pilotage_mission_core::executor::ExecutorRegistry,
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
    /// Operations of host executors, one task each, in order.
    Operations(Vec<PlannedOperation>),
}

/// One operation of a plan and the task it carries out.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedOperation {
    /// The task the operation is.
    pub task_id: String,
    /// The operation, with parameters that may name results of earlier tasks.
    pub operation: wire::MissionOperation,
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
    /// What the run's tasks that ended before it started returned, by task,
    /// for the operations that take them: the tasks a run restarted after
    /// the host restarted had carried out.
    pub results: Vec<(String, Vec<wire::MissionValue>)>,
    /// The run only returns home under an approval's recovery. It may take
    /// a lease freed after a lost control, which a run of work may not.
    pub recovery: bool,
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
    /// Give a person's answer to the operation that asked.
    Answer(OperationAnswer),
}

/// A person's answer to the question of one operation of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAnswer {
    /// The operation that asked.
    pub operation_id: String,
    /// The person confirms what was asked.
    pub accepted: bool,
    /// Who answered, as the host wrote it.
    pub answered_by: String,
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
    /// The runs a successor replaced, oldest first. Each keeps the answers
    /// still due inside its tasks' evidence windows.
    past: Vec<RunTracker>,
    detail: String,
    sequence: u64,
    changed: bool,
    /// The latest civil time the lane knows, from intents and the clock.
    clock_unix_ms: u64,
    /// The active approval expired during the run and its recovery applied.
    expired: bool,
    /// The run in progress only returns home, as a recovery.
    recovering: bool,
    /// What the run does now, as the host last said.
    execution: Option<RunExecution>,
    /// Why the run holds, when it does.
    holds: Vec<wire::MissionHoldReason>,
    /// Work the plan's revisions released, for the team that asked.
    released: Vec<wire::WorkReleased>,
    /// The latest time on the host's monotonic clock, milliseconds.
    host_ms: u64,
    /// What a restart of the host left of the run, until it is settled.
    restart: Option<restore::Restart>,
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
            past: Vec::new(),
            detail: "waiting for an objective".to_owned(),
            sequence: 0,
            changed: true,
            clock_unix_ms: 0,
            expired: false,
            recovering: false,
            execution: None,
            holds: Vec::new(),
            released: Vec::new(),
            host_ms: 0,
            restart: None,
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
            Some(Action::Hold(_)) => self.hold(),
            Some(Action::Resume(_)) => self.resume(context),
            Some(Action::Abort(_)) => self.abort(),
            Some(Action::Revise(revise)) => self.revise(revise, context),
            Some(Action::MemberDown(_)) => {
                Err("a vehicle lane has no members; send it to the team".to_owned())
            }
            Some(Action::Fetch(fetch)) => self.fetch(&fetch.digest),
            Some(Action::ResolveHandover(_)) => {
                Err("a vehicle lane has no handovers; send it to the team".to_owned())
            }
            Some(Action::Answer(answer)) => self.answer(answer),
            Some(Action::Skip(_)) => Err(
                "skipping a task needs a successor plan, which this lane does not compile yet"
                    .to_owned(),
            ),
            None => Err("the intent names no action".to_owned()),
        };
        self.changed = true;
        let (outcome, reason, effect, record) = match result {
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
                lane: self.config.name.clone(),
                proposal_record: record,
                ..wire::MissionReceipt::default()
            },
            effect,
        }
    }

    fn propose(
        &mut self,
        objective: &wire::MissionObjective,
        context: &IntentContext,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
        if self.is_flying() {
            return Err("a run is in progress; revise it instead".to_owned());
        }
        let compiled = compile::compile(objective, &self.config, context.fresh_id.clone())?;
        let record = compiled.record.clone();
        self.proposal = Some(compiled);
        self.approval = None;
        self.run = None;
        self.successor = None;
        self.revisions = 0;
        self.past.clear();
        self.holds.clear();
        self.released.clear();
        self.restart = None;
        self.state = wire::MissionLaneState::Proposed;
        self.detail = "proposed; waiting for an approval".to_owned();
        Ok((None, Some(record)))
    }

    fn approve(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
        if self.is_flying() {
            let plan = self
                .proposal
                .as_ref()
                .map(|c| c.proposal.digest.clone())
                .unwrap_or_default();
            let run = self.run.as_ref().map(RunTracker::run_id);
            let drafted = self
                .successor
                .as_ref()
                .filter(|s| s.stage == wire::MissionSuccessorState::Drafted)
                .map(|s| s.compiled.proposal.digest.clone());
            let held = Approvable {
                plan: &plan,
                run: run.as_deref(),
                drafted: drafted.as_deref(),
            };
            return match admit_approval(approval, held, context.now_unix_ms)?.0 {
                ApprovalTarget::Renewal => self.renew(approval, context),
                ApprovalTarget::Successor | ApprovalTarget::Plan => {
                    self.approve_successor(approval, context)
                }
            };
        }
        if self.state != wire::MissionLaneState::Proposed {
            return Err(format!(
                "an approval needs a proposal; the lane is {:?}",
                self.state
            ));
        }
        let compiled = self.proposal.as_ref().ok_or("no proposal to approve")?;
        let held = Approvable {
            plan: &compiled.proposal.digest,
            run: None,
            drafted: None,
        };
        admit_approval(approval, held, context.now_unix_ms)?;
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
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
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
        target::still_valid(approval, context.now_unix_ms)?;
        self.expired = false;
        self.recovering = false;
        let run_id = context.fresh_id.clone();
        self.run = Some(RunTracker::new(
            run_id.clone(),
            compiled,
            self.config.judged,
        ));
        self.state = wire::MissionLaneState::Running;
        self.detail = "running".to_owned();
        Ok((
            Some(LaneEffect::Start(Box::new(RunOrder {
                run_id,
                plan: compiled.plan.clone(),
                config: compiled.config.clone(),
                results: Vec::new(),
                recovery: false,
            }))),
            None,
        ))
    }

    /// The record of the proposal `digest` names: the plan, or a drafted
    /// revision; an empty digest names the plan.
    fn fetch(
        &self,
        digest: &[u8],
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
        let plan = self.proposal.as_ref().map(|c| &c.record);
        let revision = self.successor.as_ref().map(|s| &s.compiled.record);
        [plan, revision]
            .into_iter()
            .flatten()
            .find(|r| digest.is_empty() || r.digest == digest)
            .map(|r| (None, Some(r.clone())))
            .ok_or_else(|| "the lane holds no proposal with that digest".to_owned())
    }

    fn abort(
        &mut self,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
        if !self.is_flying() {
            return Err(format!("nothing to abort; the lane is {:?}", self.state));
        }
        self.state = wire::MissionLaneState::Aborted;
        self.successor = None;
        self.holds.clear();
        // What a restart left of the run ends with it.
        self.restart = None;
        self.detail = "aborted by intent; the vehicle stops in place".to_owned();
        if let Some(run) = self.run.as_mut() {
            run.abort(self.host_ms);
        }
        Ok((Some(LaneEffect::Abort), None))
    }

    /// Folds the run's progress in. A plan that completes ends the run,
    /// unless an admitted successor takes over: then the host starts the
    /// successor's run, the effect returned.
    pub fn on_progress(&mut self, progress: &RunProgress) -> Option<LaneEffect> {
        let flying = self.is_flying();
        let was_freed = self.control_freed();
        if progress.execution != self.execution {
            self.execution = progress.execution;
            self.changed = true;
        }
        let now_ms = self.host_ms;
        let run = self.run.as_mut()?;
        if run.on_progress(progress, now_ms) {
            self.changed = true;
        }
        if self.state == wire::MissionLaneState::HoldRequested && progress.holding {
            self.state = wire::MissionLaneState::Held;
            self.detail = "held in place".to_owned();
            self.changed = true;
        }
        // Each lease that ends without a hand-back starts the recovery once.
        if !was_freed && let Some(effect) = self.recover_from_lost_control() {
            return Some(effect);
        }
        if flying && let Some(effect) = self.at_checkpoint(progress.plan_complete) {
            return Some(effect);
        }
        let run = self.run.as_mut()?;
        if progress.plan_complete && flying {
            run.close(wire::MissionTaskExecution::Completed, now_ms);
            self.state = wire::MissionLaneState::Complete;
            self.detail = run.summary();
            self.changed = true;
        } else if progress.ended && flying {
            run.close(wire::MissionTaskExecution::Failed, now_ms);
            self.state = wire::MissionLaneState::Failed;
            self.detail = progress
                .ended_because
                .unwrap_or("the run ended before its plan completed")
                .to_owned();
            self.changed = true;
        }
        None
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
