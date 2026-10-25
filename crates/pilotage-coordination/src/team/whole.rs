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
        let member = self
            .config
            .members
            .first()
            .map(|m| m.name.clone())
            .ok_or("the team has no member")?;
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
        self.busy(Op::Answer)?;
        let member = self
            .statuses
            .iter()
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
            wire::mission_intent::Action::Answer(answer.clone()),
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
            String::new()
        } else {
            format!("{member} refused the answer: {}", receipt.reason)
        };
        self.finish(operation, accepted, reason, None)
    }
}
