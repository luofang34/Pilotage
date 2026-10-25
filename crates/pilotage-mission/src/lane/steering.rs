//! Who may steer a lane (ADR-0052). The host admits each client with or
//! without the approver grant; the grant comes from the admission secret
//! the client presented, not from the records it sends.
//!
//! Any admitted client may propose, draft a revision, start an approved
//! proposal, hold and abort: a stop must never wait for a particular
//! client. Only an approver
//! may approve, because the approval sets the envelope the lanes adapt
//! inside. A hold that an approver set stays until an approver resumes, so
//! an automated client cannot undo a human's stop.

use pilotage_protocol::wire;

/// The admitted client that sent an intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requester {
    /// The name the client gave at admission.
    pub subject: String,
    /// The client presented the approver secret, or the host has none.
    pub may_approve: bool,
}

/// The origin the host writes into a record a session sent.
fn stamp(requester: &Requester) -> wire::RecordOrigin {
    wire::RecordOrigin {
        issuer: HOST_ISSUER.to_owned(),
        subject: requester.subject.clone(),
        organization: String::new(),
    }
}

/// The lane's record of who stopped it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Steering {
    held_by_approver: bool,
}

/// Issuer of the approver origin the host writes into an approval.
pub const HOST_ISSUER: &str = "session-host";

impl Steering {
    /// Refuses an intent `requester` may not send, with the reason. An
    /// accepted approval gets the requester as its approver, so the record
    /// names the session that approved, not what that session claimed.
    ///
    /// # Errors
    ///
    /// Returns the refusal reason.
    pub fn admit(
        &self,
        intent: &mut wire::MissionIntent,
        requester: &Requester,
    ) -> Result<(), String> {
        use wire::mission_intent::Action;
        match intent.action.as_mut() {
            Some(Action::Approve(approval)) => {
                if !requester.may_approve {
                    return Err(format!(
                        "{} may not approve: approval needs the approver secret",
                        requester.subject
                    ));
                }
                approval.approver = Some(stamp(requester));
                Ok(())
            }
            Some(Action::Revise(revise))
                if !revise.share_of.is_empty() && !requester.may_approve =>
            {
                Err(format!(
                    "{} may not draft a team's share: it needs the approver secret",
                    requester.subject
                ))
            }
            Some(Action::Propose(objective))
                if !objective.delegated_by.is_empty() && !requester.may_approve =>
            {
                Err(format!(
                    "{} may not propose for a team: it needs the approver secret",
                    requester.subject
                ))
            }
            Some(Action::Revise(revise)) => {
                // Anyone may draft; the record names the session that did.
                revise.drafted_by = Some(stamp(requester));
                Ok(())
            }
            Some(Action::MemberDown(_)) if !requester.may_approve => Err(format!(
                "{} may not state that a member is down: it needs the approver secret",
                requester.subject
            )),
            Some(Action::MemberDown(down)) => {
                // The record names the session that stated it.
                down.stated_by = Some(stamp(requester));
                Ok(())
            }
            Some(Action::Answer(_)) if !requester.may_approve => Err(format!(
                "{} may not answer for a person: it needs the approver secret",
                requester.subject
            )),
            Some(Action::Answer(answer)) => {
                // The record names the session that answered.
                answer.answered_by = Some(stamp(requester));
                Ok(())
            }
            Some(Action::Resume(_)) if self.held_by_approver && !requester.may_approve => {
                Err(format!(
                    "{} may not resume: an approver held the mission",
                    requester.subject
                ))
            }
            _ => Ok(()),
        }
    }

    /// The receipt that refuses `intent` for `reason`.
    #[must_use]
    pub fn refusal(
        intent: &wire::MissionIntent,
        lane: &str,
        reason: String,
    ) -> wire::MissionReceipt {
        wire::MissionReceipt {
            request_id: intent.request_id.clone(),
            outcome: wire::MissionReceiptOutcome::Refused as i32,
            reason,
            lane: lane.to_owned(),
            ..wire::MissionReceipt::default()
        }
    }

    /// Notes an intent the lane accepted.
    pub fn accepted(&mut self, intent: &wire::MissionIntent, requester: &Requester) {
        use wire::mission_intent::Action;
        match intent.action.as_ref() {
            Some(Action::Hold(_)) => self.held_by_approver = requester.may_approve,
            Some(Action::Resume(_) | Action::Abort(_) | Action::Start(_)) => {
                self.held_by_approver = false;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
