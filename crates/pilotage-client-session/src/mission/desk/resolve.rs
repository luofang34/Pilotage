//! The intent a mission command asks for, built from what the desk holds.

use pilotage_protocol::wire;
use wire::MissionLaneState as Lane;

use pilotage_protocol::proposal_record::unknown_features;

use super::{DeskRefusal, KNOWN_FEATURES, MissionDesk, parse_hex};
use crate::mission::{MissionCommand, approval};

impl MissionDesk {
    /// The run an approval is for: the run in progress, so that a renewal
    /// names the run it renews; empty before a start.
    fn run_in_progress(&self) -> String {
        self.status
            .as_ref()
            .filter(|status| {
                matches!(
                    Lane::try_from(status.state),
                    Ok(Lane::Running | Lane::HoldRequested | Lane::Held | Lane::Uncertain)
                )
            })
            .map(|status| status.run_id.clone())
            .unwrap_or_default()
    }

    /// The intent `command` asks for at `now_ms`. The lane, not the desk,
    /// decides whether the lane's state allows it.
    ///
    /// # Errors
    ///
    /// Returns [`DeskRefusal`] when the desk lacks what the intent needs.
    pub fn resolve(
        &self,
        command: &MissionCommand,
        now_ms: u64,
    ) -> Result<wire::mission_intent::Action, DeskRefusal> {
        use wire::mission_intent::Action;
        match command {
            MissionCommand::Propose { objective } => {
                Ok(Action::Propose(objective.objective(&self.subject, now_ms)))
            }
            MissionCommand::Approve { digest, policy } => {
                let named = digest.as_deref().map(parse_hex).transpose()?;
                let proposal = match (self.approvable(), named) {
                    (Some(held), None) => held,
                    (_, Some(named)) => self.held(&named).cloned().ok_or_else(|| {
                        DeskRefusal::UnknownDigest(digest.clone().unwrap_or_default())
                    })?,
                    (None, None) if self.unseen_proposal().is_some() => {
                        return Err(DeskRefusal::NotHeld);
                    }
                    (None, None) => return Err(DeskRefusal::NoProposal),
                };
                let unknown = unknown_features(&proposal, KNOWN_FEATURES);
                if !unknown.is_empty() {
                    return Err(DeskRefusal::UnknownFeatures(unknown));
                }
                approval(&proposal, policy, &self.run_in_progress(), now_ms)
                    .ok_or(DeskRefusal::NoHeight)
            }
            MissionCommand::MemberDown { member } => {
                Ok(Action::MemberDown(wire::MissionMemberDown {
                    member: member.clone(),
                    stated_by: None,
                }))
            }
            MissionCommand::ResolveHandover { assignment_id } => {
                Ok(Action::ResolveHandover(wire::MissionResolveHandover {
                    assignment_id: assignment_id.clone(),
                    resolved_by: None,
                }))
            }
            MissionCommand::ProposeObjective { objective } => self
                .encoded_objective(objective, now_ms)
                .map(Action::Propose),
            MissionCommand::Answer { operation, accept } => {
                self.answer(operation.as_deref(), *accept)
            }
            MissionCommand::Fetch {} => Ok(Action::Fetch(wire::MissionFetch {
                digest: self.unseen_approvable().ok_or(DeskRefusal::NoProposal)?,
            })),
            MissionCommand::Start { digest } => {
                let proposal_digest = match digest {
                    Some(named) => parse_hex(named)?,
                    None => self
                        .status
                        .as_ref()
                        .map(|status| status.proposal_digest.clone())
                        .filter(|digest| !digest.is_empty())
                        .ok_or(DeskRefusal::NothingToStart)?,
                };
                Ok(Action::Start(wire::MissionStart { proposal_digest }))
            }
            MissionCommand::Revise {
                objective,
                reassign,
                finish,
                within_envelope,
            } => self.revision(
                objective.as_ref(),
                reassign.as_ref(),
                *finish,
                *within_envelope,
                now_ms,
            ),
            MissionCommand::Hold {} => Ok(Action::Hold(wire::MissionHold {})),
            MissionCommand::Resume {} => Ok(Action::Resume(wire::MissionResume {})),
            MissionCommand::Abort {} => Ok(Action::Abort(wire::MissionAbort {})),
        }
    }
}

impl MissionDesk {
    /// The objective `encoded` names, whatever its kind; one with no
    /// identity or origin of its own gets one made by this client at
    /// `now_ms`, as a survey file does.
    fn encoded_objective(
        &self,
        encoded: &str,
        now_ms: u64,
    ) -> Result<wire::MissionObjective, DeskRefusal> {
        use prost::Message as _;
        let bytes = parse_hex(encoded)
            .map_err(|_| DeskRefusal::BadObjective("not hexadecimal".to_owned()))?;
        let mut objective = wire::MissionObjective::decode(bytes.as_slice())
            .map_err(|error| DeskRefusal::BadObjective(error.to_string()))?;
        if objective.kind.is_none() {
            return Err(DeskRefusal::BadObjective("it names no kind".to_owned()));
        }
        if objective.objective_id.is_empty() {
            objective.objective_id = format!("{}-objective-{now_ms}", self.subject);
        }
        if objective.revision == 0 {
            objective.revision = 1;
        }
        if objective.origin.is_none() {
            objective.origin = Some(wire::RecordOrigin {
                issuer: "local".to_owned(),
                subject: self.subject.clone(),
                organization: String::new(),
            });
        }
        Ok(objective)
    }
}
