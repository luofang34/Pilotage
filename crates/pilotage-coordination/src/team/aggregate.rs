//! The team's view of its members' work: every member task under the
//! team's name, kept across a member's later runs, and the team status.

use std::collections::BTreeMap;

use pilotage_protocol::wire;
use wire::MissionTaskExecution as Execution;
use wire::MissionTaskJudgment as Judgment;

use super::refly::{Refly, ReflyState};
use super::{Phase, TeamLane};

/// Every task of the team, by `member/task`, with its path.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TaskBook {
    tasks: BTreeMap<String, wire::MissionTaskStatus>,
    paths: BTreeMap<String, Vec<wire::GeoPoint>>,
    /// Each member's runs of the team plan: its first run, and the runs its revisions
    /// replaced it with, each naming its predecessor. Other runs are re-flights.
    team_runs: BTreeMap<String, Vec<String>>,
    /// Runs of lines a revision moved to a member, by run, with the member.
    adopted: BTreeMap<String, String>,
}

fn key(member: &str, task: &str) -> String {
    format!("{member}/{task}")
}

/// A member task's name as the team shows it.
fn display(member: &str, name: &str) -> String {
    format!("{member}/{name}")
}

impl TaskBook {
    /// Seeds the book from the member proposals.
    pub(super) fn plan(&mut self, proposals: &BTreeMap<String, wire::MissionProposal>) {
        self.tasks.clear();
        self.paths.clear();
        self.team_runs.clear();
        self.adopted.clear();
        for (member, proposal) in proposals {
            for task in &proposal.tasks {
                let id = key(member, &task.task_id);
                self.paths.insert(id.clone(), task.path.clone());
                self.tasks.insert(
                    id.clone(),
                    wire::MissionTaskStatus {
                        task_id: id,
                        name: display(member, &task.name),
                        kind: task.kind,
                        state: wire::MissionTaskState::Pending as i32,
                        evidence: None,
                        progress: 0.0,
                        member: member.clone(),
                        reflown: 0,
                        path: task.path.clone(),
                        execution: Execution::Pending as i32,
                        judgment: Judgment::Undetermined as i32,
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
        if !self.follows(member, status, reflying) {
            return;
        }
        for task in &status.tasks {
            let id = key(member, &task.task_id);
            // A revision's tasks, and the tasks of moved lines, are new.
            let entry = self
                .tasks
                .entry(id.clone())
                .or_insert_with(|| wire::MissionTaskStatus {
                    task_id: id.clone(),
                    name: display(member, &task.name),
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
            entry.execution = task.execution;
            entry.judgment = task.judgment;
            entry.evidence.clone_from(&task.evidence);
            entry.progress = task.progress;
            // A member team reports the lines it flew again itself.
            entry.reflown = entry.reflown.max(task.reflown);
        }
    }

    /// Whether the book follows `member`'s run in `status`: its first run of the team plan,
    /// a run that continues one the book follows, as the status names its predecessor,
    /// or a run of lines moved to it. A re-flight's run is followed by its re-flight.
    fn follows(&mut self, member: &str, status: &wire::MissionStatus, reflying: bool) -> bool {
        if let Some(owner) = self.adopted.get(&status.run_id) {
            return owner == member;
        }
        let runs = self.team_runs.entry(member.to_owned()).or_default();
        if runs.contains(&status.run_id) {
            return true;
        }
        if (runs.is_empty() && !reflying) || runs.contains(&status.predecessor_run_id) {
            runs.push(status.run_id.clone());
            return true;
        }
        false
    }

    /// Follows `member`'s run `run_id` of moved lines.
    pub(super) fn adopt(&mut self, member: &str, run_id: &str) {
        self.adopted.insert(run_id.to_owned(), member.to_owned());
    }

    /// The member's survey lines not yet flown, nor in flight, with their
    /// book identifiers and paths.
    pub(super) fn unflown_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| t.execution == Execution::Pending as i32)
            .filter_map(|t| Some((t.task_id.clone(), self.paths.get(&t.task_id)?.clone())))
            .collect()
    }

    /// The member's survey lines not yet flown, the one in flight included.
    pub(super) fn unfinished_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| {
                t.execution == Execution::Pending as i32 || t.execution == Execution::Active as i32
            })
            .filter_map(|t| Some((t.task_id.clone(), self.paths.get(&t.task_id)?.clone())))
            .collect()
    }

    /// Skips the member's lines on `paths`: another member flies them.
    pub(super) fn skip_paths(&mut self, member: &str, paths: &[Vec<wire::GeoPoint>]) {
        for (id, task) in &mut self.tasks {
            if task.member == member && self.paths.get(id).is_some_and(|p| paths.contains(p)) {
                task.state = wire::MissionTaskState::Skipped as i32;
                task.execution = Execution::Skipped as i32;
                task.judgment = Judgment::Undetermined as i32;
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

    /// The member's lines whose execution ended without meeting their
    /// evidence, with their paths.
    pub(super) fn incomplete_lines(&self, member: &str) -> Vec<(String, Vec<wire::GeoPoint>)> {
        self.tasks
            .values()
            .filter(|t| t.member == member && t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| ended(t) && t.judgment == Judgment::NotMet as i32)
            .filter_map(|t| {
                let line = t.task_id.strip_prefix(&format!("{member}/"))?.to_owned();
                Some((line, self.paths.get(&t.task_id)?.clone()))
            })
            .collect()
    }

    /// How many lines still take evidence, of members `heard` says the team
    /// still takes reports from. A skipped line takes none.
    pub(super) fn open_evidence(&self, heard: impl Fn(&str) -> bool) -> usize {
        self.tasks
            .values()
            .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter(|t| heard(&t.member) && t.execution != Execution::Skipped as i32)
            .filter(|t| {
                t.evidence
                    .as_ref()
                    .is_some_and(|e| e.collection == wire::MissionEvidenceCollection::Open as i32)
            })
            .count()
    }

    /// Whether the verdict on the member's `line` has settled: the line's
    /// execution ended and its evidence can no longer change.
    pub(super) fn settled(&self, member: &str, line: &str) -> bool {
        self.tasks.get(&key(member, line)).is_some_and(|t| {
            ended(t)
                && t.evidence
                    .as_ref()
                    .is_some_and(|e| e.collection != wire::MissionEvidenceCollection::Open as i32)
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
            entry.execution = flown.execution;
            entry.judgment = flown.judgment;
            entry.evidence.clone_from(&flown.evidence);
        }
    }

    pub(super) fn summary(&self) -> String {
        let lines: Vec<&wire::MissionTaskStatus> = self
            .tasks
            .values()
            .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .collect();
        let count = |judgment: Judgment| {
            lines
                .iter()
                .filter(|t| t.judgment == judgment as i32)
                .count()
        };
        let reflown: u32 = lines.iter().map(|t| t.reflown).sum();
        format!(
            "{} of {} lines with evidence, {} without, {} flown again",
            count(Judgment::Met),
            lines.len(),
            count(Judgment::NotMet),
            reflown
        )
    }

    fn statuses(&self) -> Vec<wire::MissionTaskStatus> {
        self.tasks.values().cloned().collect()
    }
}

/// Whether the task's execution ended, whichever way.
fn ended(task: &wire::MissionTaskStatus) -> bool {
    [
        Execution::Completed,
        Execution::Cancelled,
        Execution::Failed,
    ]
    .iter()
    .any(|e| task.execution == *e as i32)
}

/// Whether every member that may still fly reports that it holds: a
/// member whose link is lost has not, as its state is unknown.
fn members_hold(team: &TeamLane) -> bool {
    team.member_proposals
        .keys()
        .filter(|m| !team.stated_down(m))
        .all(|m| {
            !team.lost.contains_key(m)
                && team.statuses.get(m).is_some_and(|s| {
                    !matches!(
                        wire::MissionLaneState::try_from(s.state),
                        Ok(wire::MissionLaneState::Running | wire::MissionLaneState::HoldRequested)
                    )
                })
        })
}

/// How the members that may still fly move: the team holds only when every
/// one of them reports that it holds, and a member whose link is lost may
/// still fly.
fn members_motion(team: &TeamLane) -> wire::MissionLaneState {
    use wire::MissionLaneState as State;
    let mut motion = State::Running;
    let mut any_flying = false;
    for (member, status) in &team.statuses {
        if team.stated_down(member) {
            continue;
        }
        match State::try_from(status.state) {
            Ok(State::Running | State::Uncertain) => return State::Running,
            Ok(State::HoldRequested) => motion = State::HoldRequested,
            Ok(State::Held) if motion != State::HoldRequested => motion = State::Held,
            _ => continue,
        }
        if team.lost.contains_key(member) {
            return State::Running;
        }
        any_flying = true;
    }
    if any_flying { motion } else { State::Running }
}

/// Why the team holds: its operator's hold, and every reason of its members
/// that may still fly.
fn hold_reasons(team: &TeamLane, state: wire::MissionLaneState) -> Vec<i32> {
    use wire::MissionLaneState as State;
    if !matches!(state, State::Held | State::HoldRequested) {
        return Vec::new();
    }
    let mut holds = Vec::new();
    if team.phase == Phase::Held {
        holds.push(wire::MissionHoldReason::Operator as i32);
    }
    for (member, status) in &team.statuses {
        if team.stated_down(member) {
            continue;
        }
        for reason in &status.holds {
            if !holds.contains(reason) {
                holds.push(*reason);
            }
        }
    }
    holds.sort_unstable();
    holds
}

pub(super) fn team_status(team: &TeamLane) -> wire::MissionStatus {
    let state = match team.phase {
        Phase::Idle | Phase::Proposing => wire::MissionLaneState::Idle,
        Phase::Proposed | Phase::Approving => wire::MissionLaneState::Proposed,
        Phase::Admitted | Phase::Starting => wire::MissionLaneState::Admitted,
        Phase::Running | Phase::Settling => members_motion(team),
        Phase::Held if members_hold(team) => wire::MissionLaneState::Held,
        Phase::Held => wire::MissionLaneState::HoldRequested,
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
        .filter(|t| t.execution == Execution::Active as i32)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let holds = hold_reasons(team, state);
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
        members: team
            .statuses
            .iter()
            .map(|(member, status)| wire::MissionStatus {
                member_link: team.link(member) as i32,
                ..status.clone()
            })
            .collect(),
        member_link: wire::MemberLink::Unspecified as i32,
        execution: (team.phase == Phase::Settling).then_some(wire::MissionExecution {
            state: wire::MissionExecutionState::AwaitingResults as i32,
            lease_holder: 0,
        }),
        approvals_needed: team.approval_needs(&holds),
        holds,
        released: Vec::new(),
        handovers: team.handovers(),
        predecessor_run_id: String::new(),
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
