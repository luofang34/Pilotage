//! Work a team passes whole to one member (ADR-0052). An objective with no
//! area to divide, such as a report, goes to one member as an objective the
//! team made, and the team follows that member's run as any other: its
//! approval, its start, its results and its end. A question the member's
//! run asks shows in the team's status; an answer the team gets goes to the
//! member that asks, and the operator has its receipt once that member
//! answered.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;

use super::{MemberIntent, Op, Phase, TeamLane};
use crate::allocation::Assignment;

impl TeamLane {
    /// Proposes `objective` to the team's first member, whole.
    pub(super) fn propose_whole(
        &mut self,
        objective: &wire::MissionObjective,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        // The first member the team does not know as lost or down.
        let member = self
            .config
            .members
            .iter()
            .map(|m| m.name.clone())
            .find(|m| !self.lost.contains_key(m))
            .ok_or("no member of the team reports")?;
        self.reset_plan();
        self.objective = Some(objective.clone());
        self.height_m = 0.0;
        self.sweep = None;
        self.member_survey = None;
        let given = wire::MissionObjective {
            objective_id: format!("{}/{member}", context.fresh_id),
            delegated_by: self.config.name.clone(),
            ..objective.clone()
        };
        let intent = self.request(
            &member,
            Op::Propose,
            None,
            wire::mission_intent::Action::Propose(given),
        );
        self.detail = format!("waiting for {member} to propose");
        self.assignments = vec![Assignment {
            member,
            band: Vec::new(),
            lines: Vec::new(),
        }];
        self.phase = Phase::Proposing;
        Ok((vec![intent], true))
    }

    /// Passes `answer` to the member whose run asks its question.
    pub(super) fn relay_answer(
        &mut self,
        answer: &wire::MissionAnswer,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if !matches!(self.phase, Phase::Running | Phase::Held | Phase::Settling) {
            return Err(format!(
                "no run of the team asks a question; the team is {:?}",
                self.phase
            ));
        }
        self.busy(Op::Answer)?;
        // Only a member that flies the team's plan and still reports.
        let member = self
            .statuses
            .iter()
            .filter(|(m, _)| self.member_proposals.contains_key(*m) && !self.lost.contains_key(*m))
            .find(|(_, status)| {
                status.tasks.iter().any(|t| {
                    t.question
                        .as_ref()
                        .is_some_and(|q| q.operation_id == answer.operation_id)
                })
            })
            .map(|(member, _)| member.clone())
            .ok_or_else(|| format!("no member asks {}", answer.operation_id))?;
        let intent = self.request(
            &member,
            Op::Answer,
            None,
            // The team vouches for who sent it the answer, as its host wrote
            // it, after whatever that sender relayed: each sender stays named.
            wire::mission_intent::Action::Answer(wire::MissionAnswer {
                relayed_from: chain(answer),
                ..answer.clone()
            }),
        );
        self.detail = format!("passing the answer to {member}");
        Ok((vec![intent], true))
    }

    /// The member's receipt for the answer it was passed: the operator's.
    pub(super) fn on_answer_receipt(
        &mut self,
        member: &str,
        accepted: bool,
        receipt: &wire::MissionReceipt,
        operation: Option<&str>,
    ) -> Option<wire::MissionReceipt> {
        let reason = if accepted {
            self.detail = format!("{member} took the answer");
            String::new()
        } else {
            format!("{member} refused the answer: {}", receipt.reason)
        };
        self.finish(operation, accepted, reason, None)
    }
}

/// Who the team passes `answer` on for: its sender, as the team's host
/// named it, after the person that sender relayed for, when it did.
fn chain(answer: &wire::MissionAnswer) -> Option<wire::RecordOrigin> {
    let sender = answer.answered_by.clone()?;
    Some(match answer.relayed_from.as_ref() {
        Some(earlier) => wire::RecordOrigin {
            subject: format!("{} via {}", earlier.subject, sender.subject),
            ..sender
        },
        None => sender,
    })
}
