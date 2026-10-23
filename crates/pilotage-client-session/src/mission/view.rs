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
    /// The commands the lane's state offers.
    pub offered: Vec<IntentKind>,
    /// The lane's tasks.
    pub tasks: Vec<TaskView>,
    /// A team's member lanes.
    pub members: Vec<MemberView>,
}

/// One task and its evidence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskView {
    /// The task's identifier.
    pub task: String,
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
            state: wire::MissionTaskState::try_from(task.state).map_or_else(
                |_| task.state.to_string(),
                |s| format!("{s:?}").to_lowercase(),
            ),
            frames: evidence.frames,
            assessed: evidence.assessed,
            usable: evidence.usable,
            reflown: task.reflown,
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
