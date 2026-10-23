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
    /// Runs of lines a revision moved to a member, with the prefix their
    /// tasks take in the book.
    adopted: BTreeMap<String, (String, String)>,
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
        self.adopted.clear();
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
        let Some(prefix) = self.prefix_of(member, &status.run_id) else {
            return;
        };
        for task in &status.tasks {
            let id = key(member, &format!("{prefix}{}", task.task_id));
            // A revision's tasks, and the tasks of moved lines, are new.
            let entry = self
                .tasks
                .entry(id.clone())
                .or_insert_with(|| wire::MissionTaskStatus {
                    task_id: id.clone(),
                    member: member.to_owned(),
                    ..task.clone()
                });
            self.paths
                .entry(id.clone())
                .or_insert_with(|| task.path.clone());
            // This team flew the line again: the member's own verdict is
            // older and keeps arriving with every report it publishes.
            if entry.reflown > task.reflown {
                continue;
            }
            entry.state = task.state;
            entry.evidence.clone_from(&task.evidence);
            entry.progress = task.progress;
            // A member team reports the lines it flew again itself.
            entry.reflown = entry.reflown.max(task.reflown);
        }
    }

    /// The prefix the tasks of `member`'s run `run_id` take in the book:
    /// none for the member's run of the team plan and the runs its own
    /// revisions replaced it with, the adopted prefix for moved lines.
    fn prefix_of(&self, member: &str, run_id: &str) -> Option<String> {
        if let Some((owner, prefix)) = self.adopted.get(run_id) {
            return (owner == member).then(|| prefix.clone());
        }
        let original = self.original_run.get(member)?;
        let lineage = run_id == original || run_id.starts_with(&format!("{original}-rev"));
        lineage.then(String::new)
    }

    /// Follows `member`'s run `run_id` of moved lines under `prefix`.
    pub(super) fn adopt(&mut self, member: &str, run_id: &str, prefix: &str) {
        self.adopted
            .insert(run_id.to_owned(), (member.to_owned(), prefix.to_owned()));
    }

    /// The member's survey lines not yet flown, nor in flight, with their
    /// book identifiers and paths.
    pub(super) fn unflown_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| t.state == wire::MissionTaskState::Pending as i32)
            .filter_map(|t| Some((t.task_id.clone(), self.paths.get(&t.task_id)?.clone())))
            .collect()
    }

    /// The member's survey lines not yet flown, the one in flight included.
    pub(super) fn unfinished_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| {
                t.state == wire::MissionTaskState::Pending as i32
                    || t.state == wire::MissionTaskState::Active as i32
            })
            .filter_map(|t| Some((t.task_id.clone(), self.paths.get(&t.task_id)?.clone())))
            .collect()
    }

    /// Skips the member's lines on `paths`: another member flies them.
    pub(super) fn skip_paths(&mut self, member: &str, paths: &[Vec<wire::GeoPoint>]) {
        for (id, task) in &mut self.tasks {
            if task.member == member && self.paths.get(id).is_some_and(|p| paths.contains(p)) {
                task.state = wire::MissionTaskState::Skipped as i32;
            }
        }
    }

    /// The paths of the member's survey lines, flown or not.
    pub(super) fn line_paths(&self, member: &str) -> Vec<Vec<wire::GeoPoint>> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter_map(|t| self.paths.get(&t.task_id).cloned())
            .collect()
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

    /// Whether the verdict on the member's `line` has settled: judgments
    /// of its pictures can arrive after the line ends, so a verdict counts
    /// once the member flew the next line of its plan, or completed or
    /// flies home.
    pub(super) fn settled(
        &self,
        member: &str,
        line: &str,
        status: Option<&wire::MissionStatus>,
    ) -> bool {
        let ended = status.is_some_and(|s| {
            s.state == wire::MissionLaneState::Complete as i32
                || s.tasks.iter().any(|t| {
                    t.kind == wire::MissionTaskKind::ReturnHome as i32
                        && t.state == wire::MissionTaskState::Active as i32
                })
        });
        if ended {
            return true;
        }
        let Some((stem, number)) = line.rsplit_once('-') else {
            return false;
        };
        let Ok(number) = number.parse::<u32>() else {
            return false;
        };
        let next = key(member, &format!("{stem}-{:02}", number.wrapping_add(1)));
        self.tasks.get(&next).is_some_and(|t| {
            matches!(
                wire::MissionTaskState::try_from(t.state),
                Ok(wire::MissionTaskState::Done
                    | wire::MissionTaskState::Incomplete
                    | wire::MissionTaskState::Skipped)
            )
        })
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
    let survey = team
        .objective
        .as_ref()
        .and_then(|o| o.kind.as_ref())
        .map(|wire::mission_objective::Kind::Survey(survey)| survey);
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
        simulated: simulated(team),
        detail: team.detail.clone(),
        members: team.statuses.values().cloned().collect(),
        sequence: team.sequence,
        area: survey.map(|s| s.area.clone()).unwrap_or_default(),
        height_m: survey.map_or(0.0, |s| s.height_m),
        successor: team.revision_status(),
    }
}

/// Whether the team flies simulated vehicles only: the proposal's
/// assumption once there is one, before that every member's own report.
/// A team with a member that is not simulated, or with no report yet, is
/// real.
fn simulated(team: &TeamLane) -> bool {
    match team.proposal.as_ref().and_then(|p| p.assumptions.as_ref()) {
        Some(assumptions) => assumptions.simulated,
        None => !team.statuses.is_empty() && team.statuses.values().all(|s| s.simulated),
    }
}
