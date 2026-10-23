//! The desk in its JSON form, for a client that reads one object per
//! line, such as an agent: what the lane does, what it reports of each
//! member and task, and which commands its state offers.

use pilotage_protocol::wire;
use serde::Serialize;

use super::{ApprovalPolicy, FiledReceipt, IntentKind, MissionDesk};

/// The lane as the desk knows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MissionView {
    /// The lane's name.
    pub lane: String,
    /// The lane's state, such as `running`.
    pub state: String,
    /// The lane flies a simulation.
    pub simulated: bool,
    /// The status's sequence number.
    pub sequence: u64,
    /// The task in progress.
    pub active: String,
    /// The lane's words for what it does.
    pub detail: String,
    /// The proposal the desk holds, when it is the lane's.
    pub proposal: Option<ProposalView>,
    /// A revision drafted, admitted or applied to the plan in progress.
    pub successor: Option<SuccessorView>,
    /// The commands the lane's state offers.
    pub offered: Vec<IntentKind>,
    /// The lane's tasks.
    pub tasks: Vec<TaskView>,
    /// A team's member lanes.
    pub members: Vec<MemberView>,
}

/// The lane in a few fields, for a log line or an agent's step: what
/// changed is worth a line, the paths and evidence of every task are not.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MissionSummary {
    /// The lane's name.
    pub lane: String,
    /// The lane's state.
    pub state: String,
    /// The lane flies a simulation.
    pub simulated: bool,
    /// Survey lines in the plan, flown or not.
    pub lines: usize,
    /// Lines flown with their evidence.
    pub done: usize,
    /// Lines flown without enough usable pictures.
    pub incomplete: usize,
    /// Lines a revision or a skip took out.
    pub skipped: usize,
    /// Times lines were flown again.
    pub reflown: u32,
    /// The tasks in progress.
    pub active: Vec<String>,
    /// Each member lane's state and task in progress.
    pub members: Vec<MemberSummary>,
    /// A revision drafted, admitted or applied.
    pub successor: Option<SuccessorView>,
    /// The commands the lane's state offers.
    pub offered: Vec<IntentKind>,
    /// The lane's words for what it does.
    pub detail: String,
}

/// One member lane in a summary.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemberSummary {
    /// The member lane's name.
    pub lane: String,
    /// Its state.
    pub state: String,
    /// Its task in progress.
    pub active: String,
}

impl MissionView {
    /// The view in a few fields.
    #[must_use]
    pub fn summary(&self) -> MissionSummary {
        let lines: Vec<&TaskView> = self
            .tasks
            .iter()
            .filter(|t| t.kind == "survey_line")
            .collect();
        let count = |state: &str| lines.iter().filter(|t| t.state == state).count();
        MissionSummary {
            lane: self.lane.clone(),
            state: self.state.clone(),
            simulated: self.simulated,
            lines: lines.len(),
            done: count("done"),
            incomplete: count("incomplete"),
            skipped: count("skipped"),
            reflown: self.tasks.iter().map(|t| t.reflown).sum(),
            active: self
                .tasks
                .iter()
                .filter(|t| t.state == "active")
                .map(|t| t.task.clone())
                .collect(),
            members: self
                .members
                .iter()
                .map(|m| MemberSummary {
                    lane: m.lane.clone(),
                    state: m.state.clone(),
                    active: m.active.clone(),
                })
                .collect(),
            successor: self.successor.clone(),
            offered: self.offered.clone(),
            detail: self.detail.clone(),
        }
    }
}

/// A revision of the plan in progress.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SuccessorView {
    /// The digest an approval of the draft names.
    pub digest: String,
    /// What it changes, in words.
    pub summary: String,
    /// `drafted`, `admitted` or `applied`.
    pub state: String,
    /// The task whose end it waits for.
    pub applies_after: String,
    /// The session that drafted it.
    pub drafted_by: String,
}

/// One task and its evidence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskView {
    /// The task's identifier.
    pub task: String,
    /// The kind of task, such as `survey_line` or `return_home`.
    pub kind: String,
    /// The task's state, such as `done`.
    pub state: String,
    /// Pictures captured.
    pub frames: u32,
    /// Pictures judged.
    pub assessed: u32,
    /// Pictures judged usable.
    pub usable: u32,
    /// Times the task was flown again.
    pub reflown: u32,
    /// The task's path over the ground, latitude and longitude in degrees.
    pub path: Vec<[f64; 2]>,
}

/// One member lane of a team.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemberView {
    /// The member lane's name.
    pub lane: String,
    /// Its state.
    pub state: String,
    /// Its task in progress.
    pub active: String,
    /// It flies a simulation.
    pub simulated: bool,
    /// Its words for what it does.
    pub detail: String,
}

/// A proposal's identity and estimates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalView {
    /// The digest that approve and start name.
    pub digest: String,
    /// Tasks in the plan.
    pub tasks: usize,
    /// Members the plan allocates work to.
    pub members: usize,
    /// Survey lines.
    pub lines: u32,
    /// Path length, metres.
    pub path_length_m: f64,
    /// Estimated duration, seconds.
    pub duration_s: f64,
}

/// The lane's answer to one intent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReceiptView {
    /// The request it answers.
    pub request: String,
    /// What the intent asked.
    pub intent: IntentKind,
    /// The lane accepted it.
    pub accepted: bool,
    /// The lane's reason.
    pub reason: String,
    /// The lane that answered.
    pub lane: String,
}

impl MissionDesk {
    /// The desk as JSON-ready data, none before the lane published a
    /// status.
    #[must_use]
    pub fn view(&self, admitted: bool, policy: &ApprovalPolicy) -> Option<MissionView> {
        let status = self.status()?;
        let offered = self.offered(admitted, policy);
        let offered = [
            (offered.propose, IntentKind::Propose),
            (offered.approve.is_some(), IntentKind::Approve),
            (offered.start, IntentKind::Start),
            (offered.hold, IntentKind::Hold),
            (offered.resume, IntentKind::Resume),
            (offered.abort, IntentKind::Abort),
            (offered.revise, IntentKind::Revise),
        ]
        .into_iter()
        .filter_map(|(on, kind)| on.then_some(kind))
        .collect();
        Some(MissionView {
            lane: status.lane.clone(),
            state: lane_state(status.state),
            simulated: status.simulated,
            sequence: status.sequence,
            active: status.active_task.clone(),
            detail: status.detail.clone(),
            proposal: self.current_proposal().map(ProposalView::of),
            successor: status.successor.as_ref().map(|s| SuccessorView {
                digest: hex(&s.digest),
                summary: s.summary.clone(),
                state: wire::MissionSuccessorState::try_from(s.state).map_or_else(
                    |_| s.state.to_string(),
                    |state| format!("{state:?}").to_lowercase(),
                ),
                applies_after: s.applies_after.clone(),
                drafted_by: s.drafted_by.clone(),
            }),
            offered,
            tasks: status.tasks.iter().map(TaskView::of).collect(),
            members: status.members.iter().map(MemberView::of).collect(),
        })
    }
}

impl TaskView {
    fn of(task: &wire::MissionTaskStatus) -> Self {
        let evidence = task.evidence.clone().unwrap_or_default();
        Self {
            task: task.task_id.clone(),
            kind: wire::MissionTaskKind::try_from(task.kind).map_or_else(
                |_| task.kind.to_string(),
                |kind| {
                    kind.as_str_name()
                        .trim_start_matches("MISSION_TASK_KIND_")
                        .to_lowercase()
                },
            ),
            state: wire::MissionTaskState::try_from(task.state).map_or_else(
                |_| task.state.to_string(),
                |s| format!("{s:?}").to_lowercase(),
            ),
            frames: evidence.frames,
            assessed: evidence.assessed,
            usable: evidence.usable,
            reflown: task.reflown,
            path: task
                .path
                .iter()
                .map(|point| [point.latitude_deg, point.longitude_deg])
                .collect(),
        }
    }
}

impl MemberView {
    fn of(member: &wire::MissionStatus) -> Self {
        Self {
            lane: member.lane.clone(),
            state: lane_state(member.state),
            active: member.active_task.clone(),
            simulated: member.simulated,
            detail: member.detail.clone(),
        }
    }
}

impl ProposalView {
    /// The view of `proposal`.
    #[must_use]
    pub fn of(proposal: &wire::MissionProposal) -> Self {
        let estimates = proposal.estimates.unwrap_or_default();
        Self {
            digest: hex(&proposal.digest),
            tasks: proposal.tasks.len(),
            members: proposal.members.len(),
            lines: estimates.lines,
            path_length_m: estimates.path_length_m.round(),
            duration_s: estimates.duration_s.round(),
        }
    }
}

impl ReceiptView {
    /// The view of `receipt`.
    #[must_use]
    pub fn of(receipt: &FiledReceipt) -> Self {
        Self {
            request: receipt.request_id.clone(),
            intent: receipt.kind,
            accepted: receipt.accepted(),
            reason: receipt.reason.clone(),
            lane: receipt.lane.clone(),
        }
    }
}

fn lane_state(state: i32) -> String {
    wire::MissionLaneState::try_from(state)
        .map_or_else(|_| state.to_string(), |s| format!("{s:?}").to_lowercase())
}

/// Lower-case hexadecimal.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
