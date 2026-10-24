//! The intent a mission command asks for, built from what the desk holds.

use pilotage_protocol::wire;
use wire::MissionLaneState as Lane;

use super::{DeskRefusal, MissionDesk, parse_hex};
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
                approval(&proposal, policy, &self.run_in_progress(), now_ms)
                    .ok_or(DeskRefusal::NoHeight)
            }
            MissionCommand::MemberDown { member } => {
                Ok(Action::MemberDown(wire::MissionMemberDown {
                    member: member.clone(),
                }))
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
