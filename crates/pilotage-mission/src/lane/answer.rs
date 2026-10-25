//! A person's answer to the question an operation of the run asks
//! (ADR-0052). The answer names the operation and binds the digest of the
//! subject the person saw, so an answer to a subject that changed is
//! refused. The host takes it only from a session with the approver secret;
//! the lane passes it to the operation, which reports what came of it.

use pilotage_protocol::wire;

use super::{Lane, LaneEffect, OperationAnswer};

impl Lane {
    pub(super) fn answer(
        &mut self,
        answer: &wire::MissionAnswer,
    ) -> Result<(Option<LaneEffect>, Option<wire::MissionProposalRecord>), String> {
        if !self.is_flying() {
            return Err(format!(
                "no run asks a question; the lane is {:?}",
                self.state
            ));
        }
        let run = self.run.as_mut().ok_or("no run asks a question")?;
        let (index, asked) = run
            .take_question(&answer.operation_id)
            .ok_or_else(|| format!("the operation {} asks no question now", answer.operation_id))?;
        if asked.subject_digest != answer.subject_digest {
            // The question stays: the answer was for another subject.
            run.ask_again(index, asked);
            return Err("the answer is for another subject than the one asked".to_owned());
        }
        run.answered(&answer.operation_id);
        let sender = answer
            .answered_by
            .as_ref()
            .map(|origin| origin.subject.clone())
            .unwrap_or_default();
        // A relay names the person it vouches for; the record names both.
        let answered_by = match answer.relayed_from.as_ref() {
            Some(person) => format!("{} via {sender}", person.subject),
            None => sender,
        };
        self.detail = format!(
            "{} {}",
            if answered_by.is_empty() {
                "a person"
            } else {
                &answered_by
            },
            if answer.accepted {
                "confirmed"
            } else {
                "declined"
            }
        );
        Ok((
            Some(LaneEffect::Answer(OperationAnswer {
                operation_id: answer.operation_id.clone(),
                accepted: answer.accepted,
                answered_by,
            })),
            None,
        ))
    }
}
