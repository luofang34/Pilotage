//! The team lane: propose by decomposition, approve by delegation, start
//! and steer by fan-out, and adapt within the team's approval envelope.

mod aggregate;
mod refly;

use std::collections::BTreeMap;

use pilotage_mission::{IntentContext, SurveyDefaults, proposal_digest};
use pilotage_protocol::wire;

use crate::allocation::{Assignment, Sweep, allocate, sweep_of};
use aggregate::TaskBook;
use refly::Refly;

/// One member lane: a vehicle's or another team's.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// The member's lane name.
    pub name: String,
    /// Where the member launches; its plans return there.
    pub launch: wire::GeoPoint,
    /// The launch point's altitude, metres.
    pub launch_altitude_m: f64,
}

/// The team's configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamConfig {
    /// The team lane's name.
    pub name: String,
    /// The members, in no particular order.
    pub members: Vec<Member>,
    /// The values an objective's zero fields take.
    pub defaults: SurveyDefaults,
    /// The recovery the team applies when it cannot continue.
    pub recovery_policy: String,
}

/// An intent the host sends to one member lane.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberIntent {
    /// The member's lane name.
    pub member: String,
    /// The intent.
    pub intent: wire::MissionIntent,
}

/// What the host does after an input: the operator's receipt when it is
/// decided, and the member intents to send.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TeamReply {
    /// The receipt for the operator's intent, once the members answered.
    pub receipt: Option<wire::MissionReceipt>,
    /// Intents for member lanes.
    pub intents: Vec<MemberIntent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Proposing,
    Proposed,
    Approving,
    Admitted,
    Starting,
    Running,
    Held,
    Complete,
    Aborted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Propose,
    Approve,
    Start,
}

/// A member intent awaiting its receipt.
#[derive(Debug, Clone, PartialEq)]
struct Pending {
    member: String,
    op: Op,
    /// The re-flight it belongs to, or `None` for the team's own step.
    refly: Option<usize>,
}

/// The coordinator's lane.
#[derive(Debug)]
pub struct TeamLane {
    config: TeamConfig,
    phase: Phase,
    objective: Option<wire::MissionObjective>,
    sweep: Option<Sweep>,
    height_m: f64,
    assignments: Vec<Assignment>,
    member_proposals: BTreeMap<String, wire::MissionProposal>,
    proposal: Option<wire::MissionProposal>,
    approval: Option<wire::MissionApproval>,
    statuses: BTreeMap<String, wire::MissionStatus>,
    book: TaskBook,
    pending: BTreeMap<String, Pending>,
    operator_request: Option<String>,
    reflies: Vec<Refly>,
    requests: u64,
    detail: String,
    sequence: u64,
    changed: bool,
}

impl TeamLane {
    /// An idle team lane.
    #[must_use]
    pub fn new(config: TeamConfig) -> Self {
        Self {
            config,
            phase: Phase::Idle,
            objective: None,
            sweep: None,
            height_m: 0.0,
            assignments: Vec::new(),
            member_proposals: BTreeMap::new(),
            proposal: None,
            approval: None,
            statuses: BTreeMap::new(),
            book: TaskBook::default(),
            pending: BTreeMap::new(),
            operator_request: None,
            reflies: Vec::new(),
            requests: 0,
            detail: "waiting for an objective".to_owned(),
            sequence: 0,
            changed: true,
        }
    }

    /// Answers the operator's intent. A propose, approve or start is
    /// answered once the members answered.
    pub fn on_intent(
        &mut self,
        intent: &wire::MissionIntent,
        context: &IntentContext,
    ) -> TeamReply {
        use wire::mission_intent::Action;
        self.changed = true;
        let id = intent.request_id.clone();
        let result = match intent.action.as_ref() {
            Some(Action::Propose(objective)) => self.propose(objective, context),
            Some(Action::Approve(approval)) => self.approve(approval, context),
            Some(Action::Start(start)) => self.start(start),
            Some(Action::Hold(_)) => self.steer(
                &[Phase::Running],
                Phase::Held,
                Action::Hold(wire::MissionHold {}),
            ),
            Some(Action::Resume(_)) => self.steer(
                &[Phase::Held],
                Phase::Running,
                Action::Resume(wire::MissionResume {}),
            ),
            Some(Action::Abort(_)) => self.steer(
                &[Phase::Starting, Phase::Running, Phase::Held],
                Phase::Aborted,
                Action::Abort(wire::MissionAbort {}),
            ),
            Some(Action::Skip(_)) => {
                Err("a team skips a task through a successor; not supported yet".to_owned())
            }
            None => Err("the intent names no action".to_owned()),
        };
        match result {
            Ok((intents, deferred)) => {
                if deferred {
                    self.operator_request = Some(id);
                    TeamReply {
                        receipt: None,
                        intents,
                    }
                } else {
                    TeamReply {
                        receipt: Some(self.receipt(id, true, String::new(), None)),
                        intents,
                    }
                }
            }
            Err(reason) => TeamReply {
                receipt: Some(self.receipt(id, false, reason, None)),
                intents: Vec::new(),
            },
        }
    }

    fn propose(
        &mut self,
        objective: &wire::MissionObjective,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if !matches!(
            self.phase,
            Phase::Idle | Phase::Proposed | Phase::Complete | Phase::Aborted | Phase::Failed
        ) {
            return Err(format!(
                "the team is {:?}; a new objective needs a successor plan",
                self.phase
            ));
        }
        let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind.as_ref() else {
            return Err("the objective is not a survey".to_owned());
        };
        if survey.area.len() < 3 {
            return Err("the survey area needs at least three vertices".to_owned());
        }
        let defaults = self.config.defaults;
        let pick = |value: f64, default: f64| if value > 0.0 { value } else { default };
        self.height_m = pick(survey.height_m, defaults.height_m);
        let fov = pick(survey.across_track_fov_deg, defaults.across_track_fov_deg);
        let sidelap = pick(survey.sidelap, defaults.sidelap);
        let sweep = sweep_of(survey, self.height_m, fov, sidelap);
        let launches: Vec<(String, wire::GeoPoint)> = self
            .config
            .members
            .iter()
            .map(|m| (m.name.clone(), m.launch))
            .collect();
        let assignments = allocate(
            &survey.area,
            &launches,
            sweep.bearing_deg,
            sweep.footprint_m,
            sweep.spacing_m,
        );
        if assignments.is_empty() {
            return Err("no member could take a band of the area".to_owned());
        }
        self.reset_plan();
        let mut intents = Vec::new();
        for assignment in &assignments {
            let Some(member) = self.member(&assignment.member) else {
                continue;
            };
            let mut member_survey = survey.clone();
            member_survey.area.clone_from(&assignment.band);
            member_survey.launch = Some(member.launch);
            member_survey.launch_altitude_m = member.launch_altitude_m;
            member_survey.height_m = self.height_m;
            member_survey.across_track_fov_deg = fov;
            member_survey.sidelap = sidelap;
            member_survey.line_bearing_deg = Some(sweep.bearing_deg);
            member_survey.return_to_launch = true;
            let member_objective = wire::MissionObjective {
                objective_id: format!("{}/{}", context.fresh_id, member.name),
                origin: objective.origin.clone(),
                revision: 1,
                previous_digest: Vec::new(),
                title: format!("{}: lines {:?}", objective.title, assignment.lines),
                kind: Some(wire::mission_objective::Kind::Survey(member_survey)),
            };
            intents.push(self.request(
                &assignment.member,
                Op::Propose,
                None,
                wire::mission_intent::Action::Propose(member_objective),
            ));
        }
        self.objective = Some(objective.clone());
        self.sweep = Some(sweep);
        self.assignments = assignments;
        self.phase = Phase::Proposing;
        self.detail = format!("waiting for {} member proposals", intents.len());
        Ok((intents, true))
    }

    fn approve(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if self.phase != Phase::Proposed {
            return Err(format!(
                "an approval needs a team proposal; the team is {:?}",
                self.phase
            ));
        }
        let proposal = self.proposal.as_ref().ok_or("no team proposal")?;
        if approval.proposal_digest != proposal.digest {
            return Err("the approval binds another proposal digest".to_owned());
        }
        let envelope = valid_envelope(approval, context.now_unix_ms)?;
        if self.height_m > envelope.max_height_m + 1e-6 {
            return Err(format!(
                "the team flies at {:.1} m; the envelope allows {:.1} m",
                self.height_m, envelope.max_height_m
            ));
        }
        let mut intents = Vec::new();
        for assignment in self.assignments.clone() {
            let Some(member_proposal) = self.member_proposals.get(&assignment.member) else {
                continue;
            };
            let launch = self.member(&assignment.member).map(|m| m.launch);
            let member_approval = delegate(
                approval,
                member_proposal,
                &assignment.band,
                launch,
                &self.config.name,
            );
            intents.push(self.request(
                &assignment.member,
                Op::Approve,
                None,
                wire::mission_intent::Action::Approve(member_approval),
            ));
        }
        self.approval = Some(approval.clone());
        self.phase = Phase::Approving;
        self.detail = "passing the approval to the members".to_owned();
        Ok((intents, true))
    }

    fn start(&mut self, start: &wire::MissionStart) -> Result<(Vec<MemberIntent>, bool), String> {
        if self.phase != Phase::Admitted {
            return Err(format!(
                "a start needs an approved team proposal; the team is {:?}",
                self.phase
            ));
        }
        if self.proposal.as_ref().map(|p| &p.digest) != Some(&start.proposal_digest) {
            return Err("the start names another proposal than the approved one".to_owned());
        }
        let mut intents = Vec::new();
        for (member, proposal) in self.member_proposals.clone() {
            let action = wire::mission_intent::Action::Start(wire::MissionStart {
                proposal_digest: proposal.digest.clone(),
            });
            intents.push(self.request(&member, Op::Start, None, action));
        }
        self.phase = Phase::Starting;
        self.detail = "starting the members".to_owned();
        Ok((intents, true))
    }

    fn steer(
        &mut self,
        from: &[Phase],
        to: Phase,
        action: wire::mission_intent::Action,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if !from.contains(&self.phase) {
            return Err(format!("not possible while the team is {:?}", self.phase));
        }
        let members: Vec<String> = self.member_proposals.keys().cloned().collect();
        let intents = members
            .iter()
            .map(|member| {
                self.requests = self.requests.wrapping_add(1);
                MemberIntent {
                    member: member.clone(),
                    intent: wire::MissionIntent {
                        request_id: format!("{}-steer-{}", self.config.name, self.requests),
                        action: Some(action.clone()),
                    },
                }
            })
            .collect();
        self.phase = to;
        self.detail = format!("{to:?} sent to the members").to_lowercase();
        Ok((intents, false))
    }

    /// Folds a member's receipt in.
    pub fn on_member_receipt(
        &mut self,
        member: &str,
        receipt: &wire::MissionReceipt,
        context: &IntentContext,
    ) -> TeamReply {
        let Some(pending) = self.pending.remove(&receipt.request_id) else {
            return TeamReply::default();
        };
        self.changed = true;
        let accepted = receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32;
        if let Some(index) = pending.refly {
            let intents = self.on_refly_receipt(index, pending.op, accepted, receipt, context);
            return TeamReply {
                receipt: None,
                intents,
            };
        }
        if !accepted {
            return self.member_refused(member, pending.op, &receipt.reason);
        }
        match pending.op {
            Op::Propose => {
                if let Some(proposal) = receipt.proposal.clone() {
                    self.member_proposals.insert(member.to_owned(), proposal);
                }
            }
            Op::Approve | Op::Start => {}
        }
        if self
            .pending
            .values()
            .any(|p| p.refly.is_none() && p.op == pending.op)
        {
            return TeamReply::default();
        }
        self.team_step_done(pending.op, context)
    }

    fn team_step_done(&mut self, op: Op, context: &IntentContext) -> TeamReply {
        let (phase, detail, proposal) = match op {
            Op::Propose => {
                let proposal = self.compose(context);
                (
                    Phase::Proposed,
                    "proposed; waiting for an approval".to_owned(),
                    Some(proposal),
                )
            }
            Op::Approve => (
                Phase::Admitted,
                "approved by every member; waiting for start".to_owned(),
                None,
            ),
            Op::Start => (Phase::Running, "running".to_owned(), None),
        };
        self.phase = phase;
        self.detail = detail;
        let request = self.operator_request.take().unwrap_or_default();
        TeamReply {
            receipt: Some(self.receipt(request, true, String::new(), proposal)),
            intents: Vec::new(),
        }
    }

    fn member_refused(&mut self, member: &str, op: Op, reason: &str) -> TeamReply {
        self.pending.retain(|_, p| p.refly.is_some());
        let (phase, what) = match op {
            Op::Propose => (Phase::Idle, "proposal"),
            Op::Approve => (Phase::Proposed, "approval"),
            Op::Start => (Phase::Failed, "start"),
        };
        self.phase = phase;
        self.detail = format!("{member} refused the {what}: {reason}");
        let request = self.operator_request.take().unwrap_or_default();
        TeamReply {
            receipt: Some(self.receipt(request, false, self.detail.clone(), None)),
            intents: Vec::new(),
        }
    }

    /// The team proposal: the members' tasks, each named by its member,
    /// the estimates combined, and the member proposals it binds.
    fn compose(&mut self, context: &IntentContext) -> wire::MissionProposal {
        let mut tasks = Vec::new();
        let mut estimates = wire::MissionEstimates::default();
        let mut members = Vec::new();
        for (member, proposal) in &self.member_proposals {
            for task in &proposal.tasks {
                let mut task = task.clone();
                task.task_id = format!("{member}/{}", task.task_id);
                task.member.clone_from(member);
                tasks.push(task);
            }
            if let Some(e) = proposal.estimates.as_ref() {
                estimates.path_length_m += e.path_length_m;
                estimates.duration_s = estimates.duration_s.max(e.duration_s);
                estimates.energy_wh += e.energy_wh;
                estimates.lines += e.lines;
                estimates.line_spacing_m = e.line_spacing_m;
                estimates.footprint_width_m = e.footprint_width_m;
            }
            members.push(wire::MemberProposal {
                member: member.clone(),
                proposal: Some(proposal.clone()),
            });
        }
        let mut proposal = wire::MissionProposal {
            proposal_id: context.fresh_id.clone(),
            objective_id: self
                .objective
                .as_ref()
                .map(|o| o.objective_id.clone())
                .unwrap_or_default(),
            digest: Vec::new(),
            tasks,
            estimates: Some(estimates),
            assumptions: Some(wire::MissionAssumptions {
                execution_strategy: "team-of-lanes".to_owned(),
                vehicle_name: self.config.name.clone(),
                simulated: self
                    .member_proposals
                    .values()
                    .all(|p| p.assumptions.as_ref().is_some_and(|a| a.simulated)),
                ..wire::MissionAssumptions::default()
            }),
            members,
            objective: self.objective.clone(),
        };
        proposal.digest = proposal_digest(&proposal).to_vec();
        self.book.plan(&self.member_proposals);
        self.proposal = Some(proposal.clone());
        proposal
    }

    /// Folds a member's status in. Returns the member intents the team's
    /// adaptation sends: re-flights of lines flown without evidence.
    pub fn on_member_status(
        &mut self,
        status: &wire::MissionStatus,
        context: &IntentContext,
    ) -> Vec<MemberIntent> {
        self.changed = true;
        let member = status.lane.clone();
        self.book.record(status, &self.reflies);
        self.statuses.insert(member.clone(), status.clone());
        self.refly_progress(&member, status);
        let intents = self.adapt(&member, status, context);
        self.settle();
        intents
    }

    /// A member's link is gone: its state is unknown, and its work stays
    /// reserved to it.
    pub fn on_member_lost(&mut self, member: &str) {
        if let Some(status) = self.statuses.get_mut(member) {
            status.detail = "link lost; state unknown, work stays reserved".to_owned();
            self.changed = true;
        }
    }

    fn settle(&mut self) {
        if !matches!(self.phase, Phase::Running | Phase::Held)
            || self.reflies.iter().any(Refly::in_progress)
        {
            return;
        }
        let states: Vec<i32> = self
            .member_proposals
            .keys()
            .filter_map(|m| self.statuses.get(m))
            .map(|s| s.state)
            .collect();
        if states.len() < self.member_proposals.len() {
            return;
        }
        let complete = wire::MissionLaneState::Complete as i32;
        let failed = [
            wire::MissionLaneState::Failed as i32,
            wire::MissionLaneState::Aborted as i32,
        ];
        if states.iter().all(|s| *s == complete) {
            self.phase = Phase::Complete;
            self.detail = self.book.summary();
        } else if states.iter().all(|s| *s == complete || failed.contains(s)) {
            self.phase = Phase::Failed;
            self.detail = format!("a member ended without completing; {}", self.book.summary());
        }
    }

    fn request(
        &mut self,
        member: &str,
        op: Op,
        refly: Option<usize>,
        action: wire::mission_intent::Action,
    ) -> MemberIntent {
        self.requests = self.requests.wrapping_add(1);
        let request_id = format!("{}-{:?}-{}", self.config.name, op, self.requests).to_lowercase();
        self.pending.insert(
            request_id.clone(),
            Pending {
                member: member.to_owned(),
                op,
                refly,
            },
        );
        MemberIntent {
            member: member.to_owned(),
            intent: wire::MissionIntent {
                request_id,
                action: Some(action),
            },
        }
    }

    fn member(&self, name: &str) -> Option<Member> {
        self.config.members.iter().find(|m| m.name == name).cloned()
    }

    fn reset_plan(&mut self) {
        self.member_proposals.clear();
        self.proposal = None;
        self.approval = None;
        self.statuses.clear();
        self.book = TaskBook::default();
        self.pending.clear();
        self.reflies.clear();
    }

    fn receipt(
        &self,
        request_id: String,
        accepted: bool,
        reason: String,
        proposal: Option<wire::MissionProposal>,
    ) -> wire::MissionReceipt {
        let outcome = if accepted {
            wire::MissionReceiptOutcome::Accepted
        } else {
            wire::MissionReceiptOutcome::Refused
        };
        wire::MissionReceipt {
            request_id,
            outcome: outcome as i32,
            reason,
            proposal,
            lane: self.config.name.clone(),
        }
    }

    /// The team's status, its members' statuses inside it.
    #[must_use]
    pub fn status(&self) -> wire::MissionStatus {
        aggregate::team_status(self)
    }

    /// The status when it changed since the last call.
    pub fn take_changed(&mut self) -> Option<wire::MissionStatus> {
        if !self.changed {
            return None;
        }
        self.changed = false;
        self.sequence = self.sequence.wrapping_add(1);
        Some(self.status())
    }
}

/// The approval's envelope when it permits execution and has not expired.
fn valid_envelope(
    approval: &wire::MissionApproval,
    now_unix_ms: u64,
) -> Result<&wire::AdaptationEnvelope, String> {
    let envelope = approval
        .envelope
        .as_ref()
        .ok_or("the approval has no adaptation envelope")?;
    if !envelope.permit_execute {
        return Err("the approval does not permit execution".to_owned());
    }
    if envelope.expires_unix_ms <= now_unix_ms {
        return Err("the approval has expired".to_owned());
    }
    Ok(envelope)
}

/// A member's approval inside the team's: the same limits, narrowed to the
/// member's band, bound to the member's own proposal.
pub(super) fn delegate(
    team: &wire::MissionApproval,
    member: &wire::MissionProposal,
    band: &[wire::GeoPoint],
    launch: Option<wire::GeoPoint>,
    issuer: &str,
) -> wire::MissionApproval {
    let mut envelope = team.envelope.clone().unwrap_or_default();
    // An operator's bound stays the bound; without one, the member flies
    // its band, the way to and from it, and the lines' overshoot.
    if envelope.bound.is_empty() {
        envelope.bound = refly::widened(band, launch);
    }
    envelope.permit_allocate = false;
    envelope.permit_change_objective = false;
    wire::MissionApproval {
        approval_id: format!("{}/{}", team.approval_id, member.proposal_id),
        proposal_digest: member.digest.clone(),
        approver: Some(wire::RecordOrigin {
            issuer: "pilotage-coordinator".to_owned(),
            subject: issuer.to_owned(),
            organization: team
                .approver
                .as_ref()
                .map(|a| a.organization.clone())
                .unwrap_or_default(),
        }),
        envelope: Some(envelope),
        approved_unix_ms: team.approved_unix_ms,
    }
}
