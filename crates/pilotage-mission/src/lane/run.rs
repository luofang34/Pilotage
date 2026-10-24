//! The run record: which task is active, what each task acquired and what
//! the judgment service said. A survey line is complete only with its
//! acquisition evidence; a flown line without it is incomplete.

use pilotage_protocol::wire;

use super::compile::Compiled;
use super::evidence::{FrameTicket, Judgments, VerdictRefused};

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

#[derive(Debug, Clone, PartialEq)]
struct TaskTrack {
    plan: wire::MissionTaskPlan,
    state: wire::MissionTaskState,
    evidence: wire::MissionTaskEvidence,
    judgments: Judgments,
    /// The task's attempt in this run; a run flies each task once.
    attempt: u32,
}

/// The run of one approved proposal.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunTracker {
    run_id: String,
    tasks: Vec<TaskTrack>,
    min_usable: u32,
    active: Option<usize>,
    /// A judgment service judges the run's pictures.
    judged: bool,
    /// The run a revision replaced with this one.
    pub(super) predecessor: Option<String>,
}

impl RunTracker {
    pub(super) fn new(run_id: String, compiled: &Compiled, judged: bool) -> Self {
        let tasks: Vec<TaskTrack> = compiled
            .proposal
            .tasks
            .iter()
            .map(|plan| TaskTrack {
                plan: plan.clone(),
                state: wire::MissionTaskState::Pending,
                evidence: wire::MissionTaskEvidence::default(),
                judgments: Judgments::default(),
                attempt: 1,
            })
            .collect();
        Self {
            run_id,
            tasks,
            min_usable: compiled.min_usable_per_line,
            active: None,
            judged,
            predecessor: None,
        }
    }

    pub(super) fn run_id(&self) -> String {
        self.run_id.clone()
    }

    /// Projects the run's task events: a finished line takes its verdict, the way home
    /// is done, and the task in progress is active. Returns whether a task changed.
    pub(super) fn on_progress(&mut self, progress: &RunProgress) -> bool {
        let (judged, min_usable) = (self.judged, self.min_usable);
        let mut changed = false;
        for phase in &progress.finished_tasks {
            let Some(task) = self
                .task_of_phase(phase)
                .and_then(|i| self.tasks.get_mut(i))
            else {
                continue;
            };
            if matches!(
                task.state,
                wire::MissionTaskState::Pending | wire::MissionTaskState::Active
            ) {
                task.state = if is_line(&task.plan) {
                    verdict(task, judged, min_usable)
                } else {
                    wire::MissionTaskState::Done
                };
                changed = true;
            }
        }
        let active = progress
            .active_task
            .as_deref()
            .and_then(|phase| self.task_of_phase(phase));
        if let Some(task) = active.and_then(|i| self.tasks.get_mut(i))
            && task.state == wire::MissionTaskState::Pending
        {
            task.state = wire::MissionTaskState::Active;
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
        if !is_line(&task.plan) {
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
        let need = usize::try_from(self.min_usable).unwrap_or(usize::MAX);
        self.judged
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
    /// it was not judged. Returns whether the task changed.
    pub(super) fn on_assessment(
        &mut self,
        ticket: &FrameTicket,
        assessment: Option<FrameAssessment>,
    ) -> Result<bool, VerdictRefused> {
        let (judged, min_usable) = (self.judged, self.min_usable);
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
        if matches!(
            task.state,
            wire::MissionTaskState::Done | wire::MissionTaskState::Incomplete
        ) {
            task.state = verdict(task, judged, min_usable);
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

    /// Ends the run: lines already flown get their verdict, the way home
    /// is done when it was active, nothing else changes.
    pub(super) fn close(&mut self) {
        for (index, task) in self.tasks.iter_mut().enumerate() {
            if task.state == wire::MissionTaskState::Active {
                task.state = if is_line(&task.plan) {
                    verdict(task, self.judged, self.min_usable)
                } else {
                    wire::MissionTaskState::Done
                };
            }
            if Some(index) == self.active && !is_line(&task.plan) {
                task.state = wire::MissionTaskState::Done;
            }
        }
        self.active = None;
    }

    /// Closes an aborted run: the task in progress was cut short, so it
    /// stays unfinished whatever evidence it had, and a pending task stays
    /// pending.
    pub(super) fn close_aborted(&mut self) {
        if let Some(task) = self.active.and_then(|index| self.tasks.get_mut(index)) {
            task.state = wire::MissionTaskState::Incomplete;
        }
        for task in &mut self.tasks {
            if task.state == wire::MissionTaskState::Active {
                task.state = wire::MissionTaskState::Incomplete;
            }
        }
        self.active = None;
    }

    /// Survey lines flown so far, with or without their evidence.
    pub(super) fn flown_lines(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| is_line(&t.plan))
            .filter(|t| {
                matches!(
                    t.state,
                    wire::MissionTaskState::Done | wire::MissionTaskState::Incomplete
                )
            })
            .count()
    }

    /// The vehicle flies along a survey line, not toward one.
    pub(super) fn on_line(&self) -> bool {
        self.active
            .and_then(|index| self.tasks.get(index))
            .is_some_and(|task| is_line(&task.plan))
    }

    /// Every line is flown and the run flies home.
    pub(super) fn flying_home(&self) -> bool {
        self.active
            .and_then(|index| self.tasks.get(index))
            .is_some_and(|task| !is_line(&task.plan))
    }

    /// The statuses a successor leaves behind: what was flown keeps its
    /// verdict, and every task not flown is skipped, superseded.
    pub(super) fn superseded_statuses(&self) -> Vec<wire::MissionTaskStatus> {
        let mut statuses = self.task_statuses();
        for status in &mut statuses {
            if matches!(
                wire::MissionTaskState::try_from(status.state),
                Ok(wire::MissionTaskState::Pending | wire::MissionTaskState::Active)
            ) {
                status.state = wire::MissionTaskState::Skipped as i32;
            }
            // A replaced run takes no more verdicts.
            if let Some(evidence) = status.evidence.as_mut()
                && evidence.state == wire::MissionEvidenceState::Pending as i32
            {
                evidence.state = wire::MissionEvidenceState::NotJudged as i32;
            }
        }
        statuses
    }

    /// The tasks the run has not begun.
    pub(super) fn not_begun(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.state == wire::MissionTaskState::Pending)
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
            .map(|task| wire::MissionTaskStatus {
                task_id: task.plan.task_id.clone(),
                name: task.plan.name.clone(),
                kind: task.plan.kind,
                state: task.state as i32,
                evidence: Some(wire::MissionTaskEvidence {
                    state: task
                        .judgments
                        .state(&task.evidence, self.judged, self.min_usable)
                        as i32,
                    ..task.evidence.clone()
                }),
                progress: match task.state {
                    wire::MissionTaskState::Done | wire::MissionTaskState::Incomplete => 1.0,
                    _ => 0.0,
                },
                member: task.plan.member.clone(),
                reflown: 0,
                path: task.plan.path.clone(),
            })
            .collect()
    }

    /// One line for the status detail when the run completes.
    pub(super) fn summary(&self) -> String {
        let lines = self.tasks.iter().filter(|t| is_line(&t.plan));
        let done = lines
            .clone()
            .filter(|t| t.state == wire::MissionTaskState::Done)
            .count();
        let incomplete = lines
            .filter(|t| t.state == wire::MissionTaskState::Incomplete)
            .count();
        format!("complete: {done} lines with evidence, {incomplete} lines without")
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
        })
        .collect()
}

fn is_line(plan: &wire::MissionTaskPlan) -> bool {
    plan.kind == wire::MissionTaskKind::SurveyLine as i32
}

/// A flown line is done with its evidence. A run with a judgment service
/// needs enough pictures judged usable, every sent picture answered; a
/// line nobody judged is not usable. A run without one counts captured
/// pictures, and its evidence reads as not judged.
fn verdict(task: &TaskTrack, judged: bool, min_usable: u32) -> wire::MissionTaskState {
    let done = if judged {
        task.judgments.state(&task.evidence, judged, min_usable) == wire::MissionEvidenceState::Met
    } else {
        task.evidence.frames >= min_usable
    };
    if done {
        wire::MissionTaskState::Done
    } else {
        wire::MissionTaskState::Incomplete
    }
}
