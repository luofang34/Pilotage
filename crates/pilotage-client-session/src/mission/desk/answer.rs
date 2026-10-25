//! The objective that asks a person about a value, the questions a
//! lane's tasks ask, and the answer to one. An
//! answer names the operation and binds the digest of the subject the
//! client last saw, so the lane refuses it when the subject changed.

use pilotage_protocol::wire;

use super::{DeskRefusal, MissionDesk};

impl MissionDesk {
    /// The questions the lane's tasks ask now, as its last status shows them.
    #[must_use]
    pub fn questions(&self) -> Vec<&wire::MissionQuestion> {
        self.status
            .as_ref()
            .map(|status| {
                status
                    .tasks
                    .iter()
                    .filter_map(|t| t.question.as_ref())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// An objective that reads `source`, asks `question` about the value and
    /// writes a report of it, made by this client at `now_ms`.
    pub(super) fn report_objective(
        &self,
        source: &str,
        question: &str,
        now_ms: u64,
    ) -> wire::MissionObjective {
        wire::MissionObjective {
            objective_id: format!("{}-report-{now_ms}", self.subject),
            revision: 1,
            title: question.to_owned(),
            kind: Some(wire::mission_objective::Kind::Report(
                wire::ReportObjective {
                    source: source.to_owned(),
                    question: question.to_owned(),
                },
            )),
            ..wire::MissionObjective::default()
        }
    }

    /// The answer `accept` to the question of `operation`, or to the only
    /// question asked when none is named.
    pub(super) fn answer(
        &self,
        operation: Option<&str>,
        accept: bool,
    ) -> Result<wire::mission_intent::Action, DeskRefusal> {
        let asked = self.questions();
        let question = match operation {
            Some(id) => asked.into_iter().find(|q| q.operation_id == id),
            None if asked.len() == 1 => asked.into_iter().next(),
            None => None,
        }
        .ok_or_else(|| DeskRefusal::NoQuestion(operation.unwrap_or("none named").to_owned()))?;
        Ok(wire::mission_intent::Action::Answer(wire::MissionAnswer {
            operation_id: question.operation_id.clone(),
            subject_digest: question.subject_digest.clone(),
            accepted: accept,
            answered_by: None,
            relayed_from: None,
        }))
    }
}
