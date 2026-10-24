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

    /// Projects the run's task events: a finished task completed, and the task in
    /// progress is active. Returns whether a task changed.
    pub(super) fn on_progress(&mut self, progress: &RunProgress) -> bool {
        let mut changed = false;
        for phase in &progress.finished_tasks {
            let Some(task) = self
                .task_of_phase(phase)
                .and_then(|i| self.tasks.get_mut(i))
            else {
                continue;
            };
            if matches!(task.execution, Execution::Pending | Execution::Active) {
                task.execution = Execution::Completed;
                changed = true;
            }
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
        changed
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
    /// execution ended. Returns whether the task changed.
    pub(super) fn on_assessment(
        &mut self,
        ticket: &FrameTicket,
        assessment: Option<FrameAssessment>,
    ) -> Result<bool, VerdictRefused> {
        let task = self.task_of(ticket)?;
        task.judgments.answer(ticket)?;
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
    pub(super) fn close(&mut self, outcome: Execution) {
        for task in &mut self.tasks {
            task.end_active(outcome);
        }
        self.active = None;
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

    /// The statuses a successor leaves behind: a task not begun is skipped,
    /// one in progress is cancelled, and what was flown keeps its outcome.
    pub(super) fn superseded_statuses(&self) -> Vec<wire::MissionTaskStatus> {
        let mut superseded = self.clone();
        for task in &mut superseded.tasks {
            if matches!(task.execution, Execution::Pending | Execution::Unspecified) {
                task.execution = Execution::Skipped;
            }
            task.end_active(Execution::Cancelled);
        }
        let mut statuses = superseded.task_statuses();
        for status in &mut statuses {
            // A replaced run takes no more verdicts.
            if let Some(evidence) = status.evidence.as_mut()
                && evidence.state == wire::MissionEvidenceState::Pending as i32
            {
                evidence.state = wire::MissionEvidenceState::NotJudged as i32;
            }
        }
        statuses
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
        })
        .collect()
}
