//! One task of a run as three facts: how its execution stands, whether its
//! evidence can still change, and whether the evidence meets the objective.
//! Run events set the execution; evidence never changes it. The judgment
//! and the display state follow from the two.
//!
//! The evidence window of a task opens when its execution ends: from then
//! on the task takes no pictures, so the pictures awaiting an answer are
//! all it can still get. When the window closes with answers outstanding,
//! the task is sealed: the judgment uses the answers that arrived, and an
//! answer after that is only counted as late.

use pilotage_protocol::wire;
use wire::MissionTaskExecution as Execution;
use wire::MissionTaskJudgment as Judgment;

use super::super::evidence::Judgments;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::lane) struct TaskTrack {
    pub plan: wire::MissionTaskPlan,
    pub execution: Execution,
    pub evidence: wire::MissionTaskEvidence,
    pub judgments: Judgments,
    /// The task's attempt in this run; a run flies each task once.
    pub attempt: u32,
    /// When its execution ended, on the host's monotonic clock, milliseconds.
    pub ended_at_ms: Option<u64>,
    /// The evidence window closed; the task takes no more answers.
    pub sealed: bool,
    /// The window closed with answers outstanding.
    closed: bool,
}

/// What the evidence of a run's lines is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::lane) struct EvidenceRule {
    /// A judgment service judges the run's pictures.
    pub judged: bool,
    /// Usable pictures, or captured ones without a judge, a line needs.
    pub min_usable: u32,
}

impl TaskTrack {
    pub(in crate::lane) fn new(plan: wire::MissionTaskPlan) -> Self {
        Self {
            plan,
            execution: Execution::Pending,
            evidence: wire::MissionTaskEvidence::default(),
            judgments: Judgments::default(),
            attempt: 1,
            ended_at_ms: None,
            sealed: false,
            closed: false,
        }
    }

    pub(in crate::lane) fn is_line(&self) -> bool {
        is_line(&self.plan)
    }

    /// The execution ended, whichever way.
    pub(in crate::lane) fn ended(&self) -> bool {
        matches!(
            self.execution,
            Execution::Completed | Execution::Cancelled | Execution::Failed
        )
    }

    /// Ends the execution with `outcome` at `now_ms` when it has not ended
    /// yet. Returns whether it changed.
    pub(in crate::lane) fn end(&mut self, outcome: Execution, now_ms: u64) -> bool {
        if self.ended() || self.execution == Execution::Skipped {
            return false;
        }
        self.execution = outcome;
        self.ended_at_ms = Some(now_ms);
        true
    }

    /// Ends the task in progress with `outcome`; a task not begun stays
    /// pending. Returns whether it changed.
    pub(in crate::lane) fn end_active(&mut self, outcome: Execution, now_ms: u64) -> bool {
        self.execution == Execution::Active && self.end(outcome, now_ms)
    }

    /// Closes the evidence window when `window_ms` passed since the
    /// execution ended at `now_ms`, or at once when `force`. Returns whether
    /// answers were outstanding.
    pub(in crate::lane) fn seal(&mut self, now_ms: u64, window_ms: u64, force: bool) -> bool {
        let Some(ended) = self.ended_at_ms else {
            return false;
        };
        if self.sealed || !(force || now_ms >= ended.saturating_add(window_ms)) {
            return false;
        }
        self.sealed = true;
        self.closed = self.judgments.outstanding() > 0;
        self.closed
    }

    /// The task still waits for answers inside its window.
    pub(in crate::lane) fn settling(&self) -> bool {
        !self.sealed && self.ended() && self.judgments.outstanding() > 0
    }

    /// Whether the task can still get evidence: it takes pictures, or a
    /// picture sent for a judgment has no answer inside the window.
    pub(in crate::lane) fn collection(&self) -> wire::MissionEvidenceCollection {
        if self.closed {
            wire::MissionEvidenceCollection::Closed
        } else if self.execution == Execution::Skipped {
            wire::MissionEvidenceCollection::Settled
        } else if !self.ended() || self.judgments.outstanding() > 0 {
            wire::MissionEvidenceCollection::Open
        } else {
            wire::MissionEvidenceCollection::Settled
        }
    }

    /// Whether the known evidence meets the objective. A line is met only
    /// when it was flown to its end with enough evidence: a line cut short
    /// does not cover its swath, whatever pictures it has. The way home is
    /// met when it was flown.
    pub(in crate::lane) fn judgment(&self, rule: EvidenceRule) -> Judgment {
        match self.execution {
            Execution::Cancelled | Execution::Failed => Judgment::NotMet,
            Execution::Completed if !self.is_line() => Judgment::Met,
            Execution::Completed => {
                if self.collection() == wire::MissionEvidenceCollection::Open {
                    Judgment::Undetermined
                } else if self.evidence_met(rule) {
                    Judgment::Met
                } else {
                    Judgment::NotMet
                }
            }
            _ => Judgment::Undetermined,
        }
    }

    /// A run with a judgment service needs enough pictures judged usable; a
    /// line nobody judged is not usable. A run without one counts captured
    /// pictures.
    fn evidence_met(&self, rule: EvidenceRule) -> bool {
        if rule.judged {
            self.evidence.assessed > 0 && self.evidence.usable >= rule.min_usable
        } else {
            self.evidence.frames >= rule.min_usable
        }
    }

    fn evidence_state(&self, rule: EvidenceRule) -> wire::MissionEvidenceState {
        if self.closed {
            return if self.evidence_met(rule) {
                wire::MissionEvidenceState::Met
            } else {
                wire::MissionEvidenceState::Missing
            };
        }
        self.judgments
            .state(&self.evidence, rule.judged, rule.min_usable)
    }

    /// The display state: done only when the execution completed and the
    /// judgment is met.
    pub(in crate::lane) fn display_state(&self, rule: EvidenceRule) -> wire::MissionTaskState {
        match self.execution {
            Execution::Pending | Execution::Unspecified => wire::MissionTaskState::Pending,
            Execution::Active => wire::MissionTaskState::Active,
            Execution::Skipped => wire::MissionTaskState::Skipped,
            Execution::Completed if self.judgment(rule) == Judgment::Met => {
                wire::MissionTaskState::Done
            }
            Execution::Completed | Execution::Cancelled | Execution::Failed => {
                wire::MissionTaskState::Incomplete
            }
        }
    }

    pub(in crate::lane) fn status(&self, rule: EvidenceRule) -> wire::MissionTaskStatus {
        wire::MissionTaskStatus {
            task_id: self.plan.task_id.clone(),
            name: self.plan.name.clone(),
            kind: self.plan.kind,
            state: self.display_state(rule) as i32,
            evidence: Some(wire::MissionTaskEvidence {
                state: self.evidence_state(rule) as i32,
                collection: self.collection() as i32,
                ..self.evidence.clone()
            }),
            progress: if self.ended() { 1.0 } else { 0.0 },
            member: self.plan.member.clone(),
            reflown: 0,
            path: self.plan.path.clone(),
            execution: self.execution as i32,
            judgment: self.judgment(rule) as i32,
        }
    }
}

pub(in crate::lane) fn is_line(plan: &wire::MissionTaskPlan) -> bool {
    plan.kind == wire::MissionTaskKind::SurveyLine as i32
}
