//! The run record: which task is active, how each task's execution ended,
//! what it acquired and what the judgment service said. Run events end a
//! task's execution; a verdict, however late, changes only its evidence and
//! the judgment that follows from it.

mod task;

use pilotage_protocol::wire;
use wire::MissionTaskExecution as Execution;
use wire::MissionTaskJudgment as Judgment;

use super::compile::Compiled;
use super::evidence::{FrameTicket, VerdictRefused};
use pilotage_mission_core::executor::{CancelSemantics, ExecutorRegistry};
use task::{EvidenceRule, TaskTrack};

/// The executor's progress, as the host reads it from the mission engine: the tasks it
/// started and finished, as the mission core names their phases.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunProgress {
    /// The task the run flies now; none between tasks.
    pub active_task: Option<String>,
    /// The tasks the run finished, in the order it finished them.
    pub finished_tasks: Vec<String>,
    /// The plan's last waypoint is captured.
    pub plan_complete: bool,
    /// The run ended without completing its plan.
    pub ended: bool,
    /// Why it ended, when the host knows.
    pub ended_because: Option<&'static str>,
    /// The vehicle holds in place, as a hold asked.
    pub holding: bool,
    /// What the run does now, when the host knows.
    pub execution: Option<RunExecution>,
    /// What the operations the run finished returned, by task.
    pub results: Vec<(String, Vec<wire::MissionValue>)>,
    /// The questions the run's operations ask a person now, by task.
    pub questions: Vec<(String, wire::MissionQuestion)>,
}

/// What a vehicle's run does now, as the host reads it from the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunExecution {
    /// The run flies its plan.
    Flying,
    /// A real vehicle's run waits for its pilot to launch it.
    AwaitingLaunch,
    /// Another principal holds the motion lease, when the run knows which.
    LeaseHeldElsewhere(Option<u64>),
    /// The run holds the vehicle in place.
    Holding,
    /// The run stopped and released the vehicle.
    Stopped,
}

impl RunExecution {
    /// The execution as a status reports it.
    #[must_use]
    pub fn wire(self) -> wire::MissionExecution {
        use wire::MissionExecutionState as State;
        let (state, lease_holder) = match self {
            Self::Flying => (State::Flying, 0),
            Self::AwaitingLaunch => (State::AwaitingLaunch, 0),
            Self::LeaseHeldElsewhere(holder) => (State::LeaseHeldElsewhere, holder.unwrap_or(0)),
            Self::Holding => (State::Holding, 0),
            Self::Stopped => (State::Stopped, 0),
        };
        wire::MissionExecution {
            state: state as i32,
            lease_holder,
        }
    }
}

/// One judgment of one picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameAssessment {
    /// The picture is usable for mapping.
    pub usable: bool,
    /// The picture shows a possible hazard to flight.
    pub hazard: bool,
}

/// The run of one approved proposal.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunTracker {
    run_id: String,
    tasks: Vec<TaskTrack>,
    rule: EvidenceRule,
    active: Option<usize>,
    /// The run a revision replaced with this one.
    pub(super) predecessor: Option<String>,
}

impl RunTracker {
    pub(super) fn new(run_id: String, compiled: &Compiled, judged: bool) -> Self {
        Self {
            run_id,
            tasks: compiled
                .proposal
                .tasks
                .iter()
                .cloned()
                .map(TaskTrack::new)
                .collect(),
            rule: EvidenceRule {
                judged,
                min_usable: compiled.min_usable_per_line,
            },
            active: None,
            predecessor: None,
        }
    }

    pub(super) fn run_id(&self) -> String {
        self.run_id.clone()
    }

    /// Projects the run's task events at `now_ms` on the host's monotonic clock: a
    /// finished task completed, and the task in progress is active. Returns whether a
    /// task changed.
    pub(super) fn on_progress(&mut self, progress: &RunProgress, now_ms: u64) -> bool {
        let mut changed = false;
        for phase in &progress.finished_tasks {
            let Some(task) = self
                .task_of_phase(phase)
                .and_then(|i| self.tasks.get_mut(i))
            else {
                continue;
            };
            changed |= task.end(Execution::Completed, now_ms);
        }
        let active = progress
            .active_task
            .as_deref()
            .and_then(|phase| self.task_of_phase(phase));
        if let Some(task) = active.and_then(|i| self.tasks.get_mut(i))
            && task.execution == Execution::Pending
        {
            task.execution = Execution::Active;
            changed = true;
        }
        changed |= active != self.active;
        self.active = active;
        changed | self.fold_operations(progress)
    }

    /// Takes the results and the questions of the run's operations, as the
    /// host reports them. Returns whether a task changed.
    fn fold_operations(&mut self, progress: &RunProgress) -> bool {
        let mut changed = false;
        for task in &mut self.tasks {
            let id = task.plan.task_id.as_str();
            if let Some((_, results)) = progress.results.iter().find(|(t, _)| t == id)
                && task.results != *results
            {
                task.results.clone_from(results);
                changed = true;
            }
            let asked = progress
                .questions
                .iter()
                .find(|(t, _)| t == id)
                .map(|(_, q)| q.clone());
            if task.question != asked {
                task.question = asked;
                changed = true;
            }
        }
        changed
    }

    /// Puts back `question` on the task `index`: an answer took it but did
    /// not answer it.
    pub(super) fn ask_again(&mut self, index: usize, question: wire::MissionQuestion) {
        if let Some(task) = self.tasks.get_mut(index) {
            task.question = Some(question);
        }
    }

    /// Takes the question of the operation `operation_id`, asked now, with
    /// its task: once answered, it is asked no more until the operation asks
    /// again.
    pub(super) fn take_question(
        &mut self,
        operation_id: &str,
    ) -> Option<(usize, wire::MissionQuestion)> {
        let index = self.tasks.iter().position(|t| {
            t.question
                .as_ref()
                .is_some_and(|q| q.operation_id == operation_id)
        })?;
        let question = self.tasks.get_mut(index)?.question.take()?;
        Some((index, question))
    }

    /// The run's task that the core's phase `phase` flies: the phases take the tasks'
    /// identities, so a phase that is no task, such as a transit to a line, names none.
    fn task_of_phase(&self, phase: &str) -> Option<usize> {
        self.tasks.iter().position(|t| t.plan.task_id == phase)
    }

    pub(super) fn on_frame(&mut self) -> Option<FrameTicket> {
        let task = self.tasks.get_mut(self.active?)?;
        if !task.is_line() || task.execution != Execution::Active {
            return None;
        }
        task.evidence.frames = task.evidence.frames.wrapping_add(1);
        Some(FrameTicket {
            run_id: self.run_id.clone(),
            task_id: task.plan.task_id.clone(),
            attempt: task.attempt,
            frame: task.evidence.frames,
        })
    }

    /// Whether the task `ticket` names needs more pictures judged: its
    /// usable pictures and the ones awaiting an answer fall short of what
    /// its evidence needs.
    pub(super) fn wants_judgment(&self, ticket: &FrameTicket) -> bool {
        let need = usize::try_from(self.rule.min_usable).unwrap_or(usize::MAX);
        self.rule.judged
            && ticket.run_id == self.run_id
            && self.tasks.iter().any(|t| {
                let usable = usize::try_from(t.evidence.usable).unwrap_or(usize::MAX);
                t.plan.task_id == ticket.task_id
                    && usable.saturating_add(t.judgments.outstanding()) < need
            })
    }

    /// Notes the picture `ticket` names as sent for judgment.
    pub(super) fn on_sent(&mut self, ticket: &FrameTicket) -> bool {
        match self.task_of(ticket) {
            Ok(task) => {
                task.judgments.sent(ticket.frame);
                true
            }
            Err(_) => false,
        }
    }

    /// Takes the verdict for the picture `ticket` names, or the news that
    /// it was not judged. It changes the task's evidence, never how its
    /// execution ended. After the task's evidence window closed, the answer
    /// is counted as late and changes nothing else. Returns whether the task
    /// changed.
    pub(super) fn on_assessment(
        &mut self,
        ticket: &FrameTicket,
        assessment: Option<FrameAssessment>,
    ) -> Result<bool, VerdictRefused> {
        let task = self.task_of(ticket)?;
        task.judgments.answer(ticket)?;
        if task.sealed {
            task.evidence.late = task.evidence.late.wrapping_add(1);
            return Err(VerdictRefused::Late {
                task_id: ticket.task_id.clone(),
                frame: ticket.frame,
            });
        }
        let evidence = &mut task.evidence;
        if let Some(assessment) = assessment {
            evidence.assessed = evidence.assessed.wrapping_add(1);
            if assessment.usable {
                evidence.usable = evidence.usable.wrapping_add(1);
            }
            if assessment.hazard {
                evidence.hazards = evidence.hazards.wrapping_add(1);
            }
        }
        Ok(true)
    }

    fn task_of(&mut self, ticket: &FrameTicket) -> Result<&mut TaskTrack, VerdictRefused> {
        if ticket.run_id != self.run_id {
            return Err(VerdictRefused::OtherRun {
                run_id: ticket.run_id.clone(),
                task_id: ticket.task_id.clone(),
            });
        }
        self.tasks
            .iter_mut()
            .find(|t| t.plan.task_id == ticket.task_id && t.attempt == ticket.attempt)
            .ok_or_else(|| VerdictRefused::UnknownTask {
                task_id: ticket.task_id.clone(),
                attempt: ticket.attempt,
            })
    }

    /// Ends the run with `outcome` for the task in progress: completed when
    /// the plan completed, cancelled by an abort, failed when the run ended
    /// without completing its plan. A task not begun stays pending.
    pub(super) fn close(&mut self, outcome: Execution, now_ms: u64) {
        for task in &mut self.tasks {
            task.end_active(outcome, now_ms);
        }
        self.active = None;
    }

    /// An abort at `now_ms`: the task in progress is cancelled, unless it is
    /// an operation that `executors` say does not stop at once. That one
    /// keeps its execution until the host reports how it ended, as it may
    /// still have its effect.
    pub(super) fn abort(&mut self, now_ms: u64, executors: &ExecutorRegistry) {
        for task in &mut self.tasks {
            let runs_on = task.plan.operation.as_ref().is_some_and(|op| {
                executors
                    .operations()
                    .iter()
                    .find(|d| d.executor == op.executor && d.operation == op.operation)
                    .is_none_or(|d| d.cancel != CancelSemantics::Immediate)
            });
            if !runs_on {
                task.end_active(Execution::Cancelled, now_ms);
            }
        }
        self.active = None;
    }

    /// A successor replaced the run at `now_ms`: a task not begun is skipped,
    /// one in progress is cancelled, and what was flown keeps its outcome and
    /// its evidence window.
    pub(super) fn supersede(&mut self, now_ms: u64) {
        for task in &mut self.tasks {
            if matches!(task.execution, Execution::Pending | Execution::Unspecified) {
                task.execution = Execution::Skipped;
            }
            task.end_active(Execution::Cancelled, now_ms);
        }
        self.active = None;
    }

    /// Closes the evidence window of each task that ended `window_ms` before
    /// `now_ms`, or of every ended task when `force`. Returns whether a task
    /// changed.
    pub(super) fn seal(&mut self, now_ms: u64, window_ms: u64, force: bool) -> bool {
        let mut changed = false;
        for task in &mut self.tasks {
            changed |= task.seal(now_ms, window_ms, force);
        }
        changed
    }

    /// A task of the run still waits for answers inside its window.
    pub(super) fn settling(&self) -> bool {
        self.tasks.iter().any(TaskTrack::settling)
    }

    /// Whether `ticket` names a picture of this run.
    pub(super) fn issued(&self, ticket: &FrameTicket) -> bool {
        ticket.run_id == self.run_id
    }

    /// Survey lines whose execution ended so far, with or without their evidence.
    pub(super) fn flown_lines(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.is_line() && t.ended())
            .count()
    }

    /// The vehicle flies along a survey line, not toward one.
    pub(super) fn on_line(&self) -> bool {
        self.active
            .and_then(|index| self.tasks.get(index))
            .is_some_and(TaskTrack::is_line)
    }

    /// Every line is flown and the run flies home.
    pub(super) fn flying_home(&self) -> bool {
        self.active
            .and_then(|index| self.tasks.get(index))
            .is_some_and(|task| !task.is_line())
    }

    /// The tasks the run has not begun.
    pub(super) fn not_begun(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.execution == Execution::Pending)
            .map(|t| t.plan.task_id.clone())
            .collect()
    }

    /// The name of the task in progress, for display.
    pub(super) fn active_task(&self) -> Option<String> {
        self.tasks.get(self.active?).map(|t| t.plan.name.clone())
    }

    pub(super) fn task_statuses(&self) -> Vec<wire::MissionTaskStatus> {
        self.tasks
            .iter()
            .map(|task| task.status(self.rule))
            .collect()
    }

    /// One line for the status detail when the run completes.
    pub(super) fn summary(&self) -> String {
        let lines = self.tasks.iter().filter(|t| t.is_line());
        let met = lines
            .clone()
            .filter(|t| t.judgment(self.rule) == Judgment::Met)
            .count();
        let not_met = lines
            .filter(|t| t.judgment(self.rule) == Judgment::NotMet)
            .count();
        format!("complete: {met} lines with evidence, {not_met} lines without")
    }
}

/// The task statuses of a plan that has not started.
pub(super) fn planned_statuses(tasks: &[wire::MissionTaskPlan]) -> Vec<wire::MissionTaskStatus> {
    tasks
        .iter()
        .map(|plan| wire::MissionTaskStatus {
            task_id: plan.task_id.clone(),
            name: plan.name.clone(),
            kind: plan.kind,
            state: wire::MissionTaskState::Pending as i32,
            evidence: None,
            progress: 0.0,
            member: plan.member.clone(),
            reflown: 0,
            path: plan.path.clone(),
            execution: Execution::Pending as i32,
            judgment: Judgment::Undetermined as i32,
            results: Vec::new(),
            question: None,
        })
        .collect()
}
