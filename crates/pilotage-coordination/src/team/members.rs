//! A team's answers from its members: their receipts, the team step they
//! complete, and the team proposal composed from the member proposals.

use pilotage_mission::{IntentContext, proposal_digest};
use pilotage_protocol::wire;

use super::{Op, Phase, TeamLane, TeamReply};

impl TeamLane {
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
        if pending.op == Op::Renew {
            return TeamReply {
                receipt: self.on_renew_receipt(member, accepted, receipt),
                intents: Vec::new(),
            };
        }
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
            Op::Approve | Op::Start | Op::Revise | Op::Renew => {}
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

    pub(super) fn team_step_done(&mut self, op: Op, context: &IntentContext) -> TeamReply {
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
            Op::Start | Op::Revise | Op::Renew => (Phase::Running, "running".to_owned(), None),
        };
        self.phase = phase;
        self.detail = detail;
        let request = self.operator_request.take().unwrap_or_default();
        TeamReply {
            receipt: Some(self.receipt(request, true, String::new(), proposal)),
            intents: Vec::new(),
        }
    }

    pub(super) fn member_refused(&mut self, member: &str, op: Op, reason: &str) -> TeamReply {
        self.pending.retain(|_, p| p.refly.is_some());
        let (phase, what) = match op {
            Op::Propose => (Phase::Idle, "proposal"),
            Op::Approve => (Phase::Proposed, "approval"),
            Op::Start | Op::Revise | Op::Renew => (Phase::Failed, "start"),
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
    pub(super) fn compose(&mut self, context: &IntentContext) -> wire::MissionProposal {
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
                task.name = format!("{member}/{}", task.name);
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
}
