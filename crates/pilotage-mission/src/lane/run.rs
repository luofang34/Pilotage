//! The run record: which task is active, what each task acquired and what
//! the judgment service said. A survey line is complete only with its
//! acquisition evidence; a flown line without it is incomplete.

use pilotage_protocol::wire;

use super::compile::Compiled;

/// The executor's progress, as the host reads it from the mission engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunProgress {
    /// Index of the waypoint the vehicle flies toward, `None` before the
    /// plan is active.
    pub target_waypoint: Option<usize>,
    /// The plan's last waypoint is captured.
    pub plan_complete: bool,
    /// The run ended without completing its plan.
    pub ended: bool,
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
}

/// The run of one approved proposal.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunTracker {
    run_id: String,
    tasks: Vec<TaskTrack>,
    lines: usize,
    min_usable: u32,
    active: Option<usize>,
}

impl RunTracker {
    pub(super) fn new(run_id: String, compiled: &Compiled) -> Self {
        let tasks: Vec<TaskTrack> = compiled
            .proposal
            .tasks
            .iter()
            .map(|plan| TaskTrack {
                plan: plan.clone(),
                state: wire::MissionTaskState::Pending,
                evidence: wire::MissionTaskEvidence::default(),
            })
            .collect();
        let lines = tasks.iter().filter(|t| is_line(&t.plan)).count();
        Self {
            run_id,
            tasks,
            lines,
            min_usable: compiled.min_usable_per_line,
            active: None,
        }
    }

    pub(super) fn run_id(&self) -> String {
        self.run_id.clone()
    }

    /// Survey line `k` is flown toward waypoint `2k + 1`; the way home
    /// toward waypoint `2 × lines`. Returns whether a task changed.
    pub(super) fn on_progress(&mut self, progress: &RunProgress) -> bool {
        let Some(target) = progress.target_waypoint else {
            return false;
        };
        let active = if target >= 2 * self.lines {
            self.tasks.iter().position(|t| !is_line(&t.plan))
        } else if target % 2 == 1 {
            Some(target / 2)
        } else {
            None
        };
        let flown_lines = if target >= 2 * self.lines {
            self.lines
        } else {
            target / 2
        };
        let mut changed = active != self.active;
        for (index, task) in self.tasks.iter_mut().enumerate() {
            if is_line(&task.plan)
                && index < flown_lines
                && task.state != wire::MissionTaskState::Done
            {
                if task.state != wire::MissionTaskState::Incomplete {
                    changed = true;
                }
                task.state = verdict(&task.evidence, self.min_usable);
            } else if Some(index) == active && task.state == wire::MissionTaskState::Pending {
                task.state = wire::MissionTaskState::Active;
                changed = true;
            }
        }
        self.active = active;
        changed
    }

    pub(super) fn on_frame(&mut self) -> Option<String> {
        let task = self.tasks.get_mut(self.active?)?;
        if !is_line(&task.plan) {
            return None;
        }
        task.evidence.frames = task.evidence.frames.wrapping_add(1);
        Some(task.plan.task_id.clone())
    }

    pub(super) fn on_assessment(&mut self, task_id: &str, assessment: FrameAssessment) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.plan.task_id == task_id) else {
            return false;
        };
        let evidence = &mut task.evidence;
        evidence.assessed = evidence.assessed.wrapping_add(1);
        if assessment.usable {
            evidence.usable = evidence.usable.wrapping_add(1);
        }
        if assessment.hazard {
            evidence.hazards = evidence.hazards.wrapping_add(1);
        }
        if matches!(
            task.state,
            wire::MissionTaskState::Done | wire::MissionTaskState::Incomplete
        ) {
            task.state = verdict(evidence, self.min_usable);
        }
        true
    }

    /// Ends the run: lines already flown get their verdict, the way home
    /// is done when it was active, nothing else changes.
    pub(super) fn close(&mut self) {
        for (index, task) in self.tasks.iter_mut().enumerate() {
            if task.state == wire::MissionTaskState::Active {
                task.state = if is_line(&task.plan) {
                    verdict(&task.evidence, self.min_usable)
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
        }
        statuses
    }

    pub(super) fn active_task(&self) -> Option<String> {
        self.tasks.get(self.active?).map(|t| t.plan.task_id.clone())
    }

    pub(super) fn task_statuses(&self) -> Vec<wire::MissionTaskStatus> {
        self.tasks
            .iter()
            .map(|task| wire::MissionTaskStatus {
                task_id: task.plan.task_id.clone(),
                kind: task.plan.kind,
                state: task.state as i32,
                evidence: Some(task.evidence.clone()),
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

/// A flown line is done with enough usable pictures: judged usable when a
/// judgment service assessed any, otherwise captured.
fn verdict(evidence: &wire::MissionTaskEvidence, min_usable: u32) -> wire::MissionTaskState {
    let usable = if evidence.assessed > 0 {
        evidence.usable
    } else {
        evidence.frames
    };
    if usable >= min_usable {
        wire::MissionTaskState::Done
    } else {
        wire::MissionTaskState::Incomplete
    }
}
