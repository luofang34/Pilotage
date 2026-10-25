//! A lane rebuilt after its host restarted (ADR-0052). The record keeps the
//! plan as approved, the approval and each task's execution, and a restart
//! replays nothing: the run runs again only when a person resumes it.
//!
//! A task whose operation may have run when the host stopped is settled by
//! what its executor declares. The host asks an executor that can tell
//! whether the operation happened. An operation whose repeat with the same
//! identity changes nothing runs again. Any other stays uncertain, and a
//! person decides. A flight is never flown again after a restart: the lane
//! cannot know where the vehicle is, so a person checks it.

use pilotage_mission_core::executor::OperationDescriptor;
use pilotage_protocol::proposal_record::SealedProposal;
use pilotage_protocol::wire;
use wire::MissionLaneState as State;
use wire::MissionTaskExecution as Execution;

use super::compile::Compiled;
use super::run::RunTracker;
use super::{Lane, LaneConfig, LaneEffect, PlannedOperation, RunOrder, RunPlan};

/// What the host found out about an operation after a restart.
#[derive(Debug, Clone, PartialEq)]
pub enum Reconciled {
    /// It happened and returned these results.
    Happened(Vec<wire::MissionValue>),
    /// It did not happen.
    DidNotHappen,
}

/// An operation the host asks its executor about after a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusQuery {
    /// The task the operation is.
    pub task_id: String,
    /// The operation's identity, the same as when it ran.
    pub operation_id: String,
    /// The operation.
    pub operation: wire::MissionOperation,
}

/// What a restart left of a run.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Restart {
    /// The tasks whose operation the host is asked about.
    queried: Vec<String>,
}

/// The identity of task `task_id`'s operation in run `run_id`: a run
/// carries out each task once, so a repeat of the same operation, such as
/// after a restart, has the same identity.
#[must_use]
pub fn operation_id(run_id: &str, task_id: &str) -> String {
    format!("{run_id}/{task_id}/1")
}

impl Lane {
    /// The facts of the run that must outlive the host, while the lane has
    /// a run.
    #[must_use]
    pub fn record(&self) -> Option<wire::MissionLaneRecord> {
        let run = self.run.as_ref()?;
        let compiled = self.proposal.as_ref()?;
        Some(wire::MissionLaneRecord {
            proposal: Some(compiled.record.clone()),
            approval: self.approval.clone(),
            run_id: run.run_id(),
            state: self.state as i32,
            tasks: run.record(),
        })
    }

    /// The lane `record` left, under `config`.
    ///
    /// # Errors
    ///
    /// When the record has no proposal or its proposal record does not open.
    pub fn restore(config: LaneConfig, record: &wire::MissionLaneRecord) -> Result<Self, String> {
        let sealed = record
            .proposal
            .clone()
            .ok_or("the lane record has no proposal")?;
        let (proposal, sealed) = SealedProposal::open(sealed)
            .map_err(|refused| format!("the lane record's proposal: {refused}"))?
            .into_parts();
        let compiled = restored_plan(proposal, sealed, &config);
        let run = RunTracker::restored(
            record.run_id.clone(),
            &compiled,
            config.judged,
            &record.tasks,
        );
        let mut lane = Self::new(config);
        lane.state = State::try_from(record.state).unwrap_or(State::Uncertain);
        lane.approval = record.approval.clone();
        lane.proposal = Some(compiled);
        lane.run = Some(run);
        lane.detail = "restored after the host restarted".to_owned();
        lane.settle_operations();
        let queried = lane.queried();
        if lane.is_flying() || !queried.is_empty() {
            lane.restart = Some(Restart { queried });
        }
        if lane.is_flying() {
            lane.state = State::Uncertain;
        }
        lane.settle_restart();
        Ok(lane)
    }

    /// The operations the host asks its executors about.
    #[must_use]
    pub fn status_queries(&self) -> Vec<StatusQuery> {
        let (Some(restart), Some(run)) = (self.restart.as_ref(), self.run.as_ref()) else {
            return Vec::new();
        };
        run.tasks()
            .iter()
            .filter(|task| restart.queried.contains(&task.plan.task_id))
            .filter_map(|task| {
                Some(StatusQuery {
                    task_id: task.plan.task_id.clone(),
                    operation_id: operation_id(&run.run_id(), &task.plan.task_id),
                    operation: task.plan.operation.clone()?,
                })
            })
            .collect()
    }

    /// Takes what the host found out about the operation of `task_id`.
    ///
    /// # Errors
    ///
    /// When the host was not asked about that task.
    pub fn reconcile(&mut self, task_id: &str, found: Reconciled) -> Result<(), String> {
        let restart = self.restart.as_mut().ok_or("the lane did not restart")?;
        let at = restart
            .queried
            .iter()
            .position(|t| t == task_id)
            .ok_or_else(|| format!("the host was not asked about {task_id}"))?;
        restart.queried.remove(at);
        let now_ms = self.host_ms;
        let task = self
            .run
            .as_mut()
            .and_then(|run| run.task_mut(task_id))
            .ok_or_else(|| format!("the run has no task {task_id}"))?;
        match found {
            Reconciled::Happened(results) => {
                task.results = results;
                task.end(Execution::Completed, now_ms);
            }
            Reconciled::DidNotHappen => task.execution = Execution::Pending,
        }
        self.changed = true;
        self.settle_restart();
        Ok(())
    }

    /// Settles each operation that had not ended by what its executor
    /// declares: queried, run again, or uncertain. A flight's tasks keep
    /// their execution.
    fn settle_operations(&mut self) {
        let flying = self.is_flying();
        let executors = self.config.executors;
        let Some(run) = self.run.as_mut() else {
            return;
        };
        for task in run.unended() {
            let Some(operation) = task.plan.operation.as_ref() else {
                continue;
            };
            // An ended run starts nothing more: only an operation it began
            // may have had its effect.
            if !flying && task.execution != Execution::Active {
                continue;
            }
            task.execution = match descriptor(&executors, operation) {
                Some(d) if d.status_query => Execution::Uncertain,
                Some(d) if d.idempotent && flying => Execution::Pending,
                _ => Execution::Uncertain,
            };
        }
    }

    /// The tasks the host is asked about: uncertain, and their executor
    /// can tell whether they happened.
    fn queried(&self) -> Vec<String> {
        let executors = self.config.executors;
        self.run
            .as_ref()
            .map(|run| {
                run.tasks()
                    .iter()
                    .filter(|task| task.execution == Execution::Uncertain)
                    .filter(|task| {
                        task.plan
                            .operation
                            .as_ref()
                            .and_then(|o| descriptor(&executors, o))
                            .is_some_and(|d| d.status_query)
                    })
                    .map(|task| task.plan.task_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Holds a restarted run of operations for a person to resume once no
    /// task is uncertain; a flight, or a run with a task a person must
    /// decide, stays uncertain.
    fn settle_restart(&mut self) {
        let Some(restart) = self.restart.as_ref() else {
            return;
        };
        // A run that ended, before the restart or by an abort since, only
        // learns how its operations ended.
        if self.state != State::Uncertain {
            if restart.queried.is_empty() {
                self.restart = None;
            }
            return;
        }
        if !restart.queried.is_empty() {
            self.detail = format!(
                "restored after the host restarted; asking whether {} operations happened",
                restart.queried.len()
            );
            return;
        }
        let flies = self.proposal.as_ref().is_none_or(Compiled::flies);
        let tasks = self.run.as_ref().map(RunTracker::tasks).unwrap_or_default();
        if flies {
            self.detail =
                "the host restarted during the flight; check the vehicle, then abort the run"
                    .to_owned();
        } else if tasks.iter().any(|t| t.execution == Execution::Uncertain) {
            self.detail = "the host restarted; whether an operation happened is not known, so check it, then abort the run".to_owned();
        } else if tasks.iter().all(|t| t.execution == Execution::Completed) {
            self.restart = None;
            self.state = State::Complete;
            self.detail = "every operation happened before the host restarted".to_owned();
        } else {
            self.state = State::Held;
            self.add_hold(wire::MissionHoldReason::Restarted);
            self.detail = "the host restarted; resume the run to carry out what is left".to_owned();
        }
    }

    /// The start of what is left of a restarted run, under its own identity
    /// so that an operation run again keeps its identity.
    pub(super) fn restart_order(&mut self) -> Option<LaneEffect> {
        self.restart.as_ref()?;
        let compiled = self.proposal.as_ref()?;
        let run = self.run.as_ref()?;
        let RunPlan::Operations(planned) = &compiled.plan else {
            return None;
        };
        let left = |p: &&PlannedOperation| {
            run.tasks()
                .iter()
                .any(|t| t.plan.task_id == p.task_id && t.execution == Execution::Pending)
        };
        let order = RunOrder {
            run_id: run.run_id(),
            plan: RunPlan::Operations(planned.iter().filter(left).cloned().collect()),
            config: compiled.config.clone(),
            results: run
                .tasks()
                .iter()
                .filter(|t| t.execution == Execution::Completed)
                .map(|t| (t.plan.task_id.clone(), t.results.clone()))
                .collect(),
        };
        self.restart = None;
        Some(LaneEffect::Start(Box::new(order)))
    }
}

/// The descriptor of `operation` in `executors`, at its version.
fn descriptor(
    executors: &pilotage_mission_core::executor::ExecutorRegistry,
    operation: &wire::MissionOperation,
) -> Option<&'static OperationDescriptor> {
    executors.operations().iter().find(|d| {
        d.executor == operation.executor
            && d.operation == operation.operation
            && d.version == operation.version
    })
}

/// The plan of `proposal` as the lane runs it: a plan of operations is
/// rebuilt from its tasks. A flight is not flown again after a restart, so
/// its plan is the way home and is never started.
fn restored_plan(
    proposal: wire::MissionProposal,
    record: wire::MissionProposalRecord,
    config: &LaneConfig,
) -> Compiled {
    let operations: Option<Vec<PlannedOperation>> = proposal
        .tasks
        .iter()
        .map(|task| {
            Some(PlannedOperation {
                task_id: task.task_id.clone(),
                operation: task.operation.clone()?,
            })
        })
        .collect();
    let survey = proposal
        .objective
        .as_ref()
        .and_then(|o| o.kind.as_ref())
        .and_then(|kind| match kind {
            wire::mission_objective::Kind::Survey(survey) => Some(survey.clone()),
            wire::mission_objective::Kind::Report(_) => None,
        });
    let mut mission = config.template.clone();
    mission.task_ids = proposal.tasks.iter().map(|t| t.task_id.clone()).collect();
    Compiled {
        plan: match operations {
            Some(operations) if !operations.is_empty() => RunPlan::Operations(operations),
            _ => RunPlan::Home,
        },
        config: mission,
        height_m: survey.as_ref().map_or(0.0, |s| s.height_m),
        min_usable_per_line: survey.map_or(1, |s| s.min_usable_frames_per_line.max(1)),
        proposal,
        record,
    }
}
