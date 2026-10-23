//! The team's view of its members' work: every member task under the
//! team's name, kept across a member's later runs, and the team status.

use std::collections::BTreeMap;

use pilotage_protocol::wire;

use super::refly::{Refly, ReflyState};
use super::{Phase, TeamLane};

/// Every task of the team, by `member/task`, with its path.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TaskBook {
    tasks: BTreeMap<String, wire::MissionTaskStatus>,
    paths: BTreeMap<String, Vec<wire::GeoPoint>>,
    /// Each member's run of the team plan; later runs are re-flights.
    original_run: BTreeMap<String, String>,
}

fn key(member: &str, task: &str) -> String {
    format!("{member}/{task}")
}

impl TaskBook {
    /// Seeds the book from the member proposals.
    pub(super) fn plan(&mut self, proposals: &BTreeMap<String, wire::MissionProposal>) {
        self.tasks.clear();
        self.paths.clear();
        self.original_run.clear();
        for (member, proposal) in proposals {
            for task in &proposal.tasks {
                let id = key(member, &task.task_id);
                self.paths.insert(id.clone(), task.path.clone());
                self.tasks.insert(
                    id.clone(),
                    wire::MissionTaskStatus {
                        task_id: id,
                        kind: task.kind,
                        state: wire::MissionTaskState::Pending as i32,
                        evidence: None,
                        progress: 0.0,
                        member: member.clone(),
                        reflown: 0,
                        path: task.path.clone(),
                    },
                );
            }
        }
    }

    /// Folds a member's status in when it reports the member's run of the
    /// team plan.
    pub(super) fn record(&mut self, status: &wire::MissionStatus, reflies: &[Refly]) {
        let member = status.lane.as_str();
        let reflying = reflies.iter().any(|r| {
            r.member == member && r.state != ReflyState::Done && r.state != ReflyState::Failed
        });
        if status.run_id.is_empty() {
            return;
        }
        if !self.original_run.contains_key(member) && !reflying {
            self.original_run
                .insert(member.to_owned(), status.run_id.clone());
        }
        if self.original_run.get(member) != Some(&status.run_id) {
            return;
        }
        for task in &status.tasks {
            if let Some(entry) = self.tasks.get_mut(&key(member, &task.task_id)) {
                entry.state = task.state;
                entry.evidence.clone_from(&task.evidence);
                entry.progress = task.progress;
            }
        }
    }

    /// The member's lines flown without evidence, with their paths.
    pub(super) fn incomplete_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.state == wire::MissionTaskState::Incomplete as i32)
            .filter_map(|t| {
                let line = t.task_id.strip_prefix(&format!("{member}/"))?.to_owned();
                Some((line, self.paths.get(&t.task_id)?.clone()))
            })
            .collect()
    }

    /// The original line takes the re-flight's verdict and evidence.
    pub(super) fn apply_refly(
        &mut self,
        member: &str,
        line: &str,
        flown: &wire::MissionTaskStatus,
    ) {
        if let Some(entry) = self.tasks.get_mut(&key(member, line)) {
            entry.reflown = entry.reflown.wrapping_add(1);
            entry.state = flown.state;
            entry.evidence.clone_from(&flown.evidence);
        }
    }

    pub(super) fn summary(&self) -> String {
        let lines: Vec<&wire::MissionTaskStatus> = self
            .tasks
            .values()
            .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .collect();
        let count = |state: wire::MissionTaskState| {
            lines.iter().filter(|t| t.state == state as i32).count()
        };
        let reflown: u32 = lines.iter().map(|t| t.reflown).sum();
        format!(
            "{} of {} lines with evidence, {} without, {} flown again",
            count(wire::MissionTaskState::Done),
            lines.len(),
            count(wire::MissionTaskState::Incomplete),
            reflown
        )
    }

    fn statuses(&self) -> Vec<wire::MissionTaskStatus> {
        self.tasks.values().cloned().collect()
    }
}

pub(super) fn team_status(team: &TeamLane) -> wire::MissionStatus {
    let state = match team.phase {
        Phase::Idle | Phase::Proposing => wire::MissionLaneState::Idle,
        Phase::Proposed | Phase::Approving => wire::MissionLaneState::Proposed,
        Phase::Admitted | Phase::Starting => wire::MissionLaneState::Admitted,
        Phase::Running => wire::MissionLaneState::Running,
        Phase::Held => wire::MissionLaneState::Held,
        Phase::Complete => wire::MissionLaneState::Complete,
        Phase::Aborted => wire::MissionLaneState::Aborted,
        Phase::Failed => wire::MissionLaneState::Failed,
    };
    let proposal = team.proposal.as_ref();
    let area = team
        .objective
        .as_ref()
        .and_then(|o| o.kind.as_ref())
        .map(|wire::mission_objective::Kind::Survey(survey)| survey.area.clone())
        .unwrap_or_default();
    let active = team
        .book
        .statuses()
        .iter()
        .filter(|t| t.state == wire::MissionTaskState::Active as i32)
        .map(|t| t.task_id.clone())
        .collect::<Vec<_>>()
        .join(", ");
    wire::MissionStatus {
        lane: team.config.name.clone(),
        state: state as i32,
        objective_id: team
            .objective
            .as_ref()
            .map(|o| o.objective_id.clone())
            .unwrap_or_default(),
        proposal_digest: proposal.map(|p| p.digest.clone()).unwrap_or_default(),
        plan_digest: Vec::new(),
        run_id: team.run_id.clone().unwrap_or_default(),
        tasks: team.book.statuses(),
        active_task: active,
        recovery_policy: team.config.recovery_policy.clone(),
        estimates: proposal.and_then(|p| p.estimates),
        simulated: proposal
            .and_then(|p| p.assumptions.as_ref())
            .is_some_and(|a| a.simulated),
        detail: team.detail.clone(),
        members: team.statuses.values().cloned().collect(),
        sequence: team.sequence,
        area,
    }
}
