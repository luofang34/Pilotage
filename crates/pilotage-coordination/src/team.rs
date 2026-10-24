//! The team lane: propose by decomposition, approve by delegation, start
//! and steer by fan-out, and adapt within the team's approval envelope.

mod aggregate;
mod intents;
mod refly;
mod revision;

use std::collections::BTreeMap;

use pilotage_mission::{IntentContext, SurveyDefaults, proposal_digest};
use pilotage_protocol::wire;

use crate::allocation::{Assignment, Sweep};
use aggregate::TaskBook;
use intents::delegate;
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
    Revise,
}

/// A member intent awaiting its receipt.
#[derive(Debug, Clone, PartialEq)]
struct Pending {
    member: String,
    op: Op,
    /// The re-flight it belongs to, or `None` for the team's own step.
    refly: Option<usize>,
    /// It is a member's share of a revision.
    revision: bool,
}

/// The coordinator's lane.
#[derive(Debug)]
pub struct TeamLane {
    config: TeamConfig,
    phase: Phase,
    objective: Option<wire::MissionObjective>,
    sweep: Option<Sweep>,
    /// The survey every member objective starts from: the team's height,
    /// sweep and spacing.
    member_survey: Option<wire::SurveyObjective>,
    height_m: f64,
    assignments: Vec<Assignment>,
    member_proposals: BTreeMap<String, wire::MissionProposal>,
    proposal: Option<wire::MissionProposal>,
    approval: Option<wire::MissionApproval>,
    statuses: BTreeMap<String, wire::MissionStatus>,
    book: TaskBook,
    pending: BTreeMap<String, Pending>,
    operator_request: Option<String>,
    /// The team's run, once started; a parent team follows it by this id.
    run_id: Option<String>,
    reflies: Vec<Refly>,
    revision: Option<revision::Revision>,
    /// Members whose link is lost, and whether a person released one:
    /// its lines went to others and the team no longer waits for it.
    lost: BTreeMap<String, bool>,
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
            member_survey: None,
            height_m: 0.0,
            assignments: Vec::new(),
            member_proposals: BTreeMap::new(),
            proposal: None,
            approval: None,
            statuses: BTreeMap::new(),
            book: TaskBook::default(),
            pending: BTreeMap::new(),
            operator_request: None,
            run_id: None,
            reflies: Vec::new(),
            revision: None,
            lost: BTreeMap::new(),
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
            Some(Action::Start(start)) => self.start(start, context),
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
            Some(Action::Revise(revise)) => return self.on_revise(id, revise, context),
            Some(Action::Fetch(fetch)) => return self.on_fetch(id, &fetch.digest),
            Some(Action::Skip(_)) => {
                Err("a team skips a task through a revision; draft one instead".to_owned())
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
        if pending.revision {
            return TeamReply {
                receipt: None,
                intents: self.on_revision_receipt(member, pending.op, accepted, receipt),
            };
        }
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
            Op::Approve | Op::Start | Op::Revise => {}
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
            Op::Start | Op::Revise => (Phase::Running, "running".to_owned(), None),
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
            Op::Start | Op::Revise => (Phase::Failed, "start"),
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
        let each: Vec<wire::MissionEstimates> = self
            .member_proposals
            .values()
            .filter_map(|p| p.estimates)
            .collect();
        let estimates = wire::MissionEstimates {
            path_length_m: each.iter().map(|e| e.path_length_m).sum(),
            duration_s: each.iter().map(|e| e.duration_s).fold(0.0, f64::max),
            energy_wh: each.iter().map(|e| e.energy_wh).sum(),
            lines: each.iter().map(|e| e.lines).sum(),
            line_spacing_m: each.first().map_or(0.0, |e| e.line_spacing_m),
            footprint_width_m: each.first().map_or(0.0, |e| e.footprint_width_m),
        };
        let mut members = Vec::new();
        for (member, proposal) in &self.member_proposals {
            for task in &proposal.tasks {
                let mut task = task.clone();
                task.task_id = format!("{member}/{}", task.task_id);
                task.member.clone_from(member);
                tasks.push(task);
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
            predecessor_digest: Vec::new(),
            summary: String::new(),
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
        if !self.reporting(&member) {
            return Vec::new();
        }
        self.book.record(status, &self.reflies);
        self.statuses.insert(member.clone(), status.clone());
        self.refly_progress(&member, status);
        let intents = self.adapt(context);
        self.settle();
        intents
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
            .filter(|m| self.lost.get(*m) != Some(&true))
            .filter_map(|m| self.statuses.get(m))
            .map(|s| s.state)
            .collect();
        let released = self.lost.values().filter(|r| **r).count();
        if states.len() + released < self.member_proposals.len() {
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
                revision: false,
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
        self.revision = None;
        // A new plan has no run until it starts: a parent following the
        // old run must not read the new plan's tasks as that run's.
        self.run_id = None;
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
