//! One client's view of one mission lane: the lane's latest status, the
//! proposal this client's latest accepted propose compiled, the answer to
//! its latest intent, and the intents not yet answered. A team lane and a
//! vehicle's lane speak the same messages, so one desk serves both.

use pilotage_protocol::wire;
use wire::MissionLaneState as Lane;

use super::{ApprovalPolicy, IntentKind, MissionCommand, approval, approved_height_m};
use crate::{ClientAction, ClientEngine};

/// Intents the desk remembers until the lane answers them; an answer to
/// an older one is not attributed.
const TRACKED_INTENTS: usize = 16;

/// Why the desk built no intent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeskRefusal {
    /// The host has not admitted the session.
    #[error("{0} not sent: the host has not admitted this session")]
    NotAdmitted(&'static str),
    /// The desk holds no proposal to approve.
    #[error("no proposal to approve: propose first")]
    NoProposal,
    /// The named digest is not the proposal the desk holds, and no
    /// stated height ceiling bounds an approval of it.
    #[error(
        "the proposal {0} is not the one this client holds; state policy.max_height_m to approve it"
    )]
    UnknownDigest(String),
    /// The text is not a hexadecimal digest.
    #[error("{0:?} is not a hexadecimal digest")]
    BadDigest(String),
    /// Neither the policy nor the objective states a height.
    #[error("no height bounds the approval: state policy.max_height_m")]
    NoHeight,
    /// No proposal is named to start.
    #[error("no approved proposal to start")]
    NothingToStart,
    /// The lane names no plan to revise.
    #[error("no plan to revise: propose first")]
    NothingToRevise,
    /// A revision must name exactly one change.
    #[error("a revision names exactly one change: objective, reassign or finish")]
    RevisionShape,
}

/// The lane's answer to one of this client's intents.
#[derive(Clone, Debug, PartialEq)]
pub struct FiledReceipt {
    /// The request it answers.
    pub request_id: String,
    /// What the intent asked.
    pub kind: IntentKind,
    /// Accepted, refused, or not yet known.
    pub outcome: wire::MissionReceiptOutcome,
    /// The lane's reason, mostly for a refusal.
    pub reason: String,
    /// The lane that answered.
    pub lane: String,
}

impl FiledReceipt {
    /// The lane accepted the intent.
    #[must_use]
    pub fn accepted(&self) -> bool {
        self.outcome == wire::MissionReceiptOutcome::Accepted
    }
}

/// Which commands the lane's state offers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offered {
    /// A new objective may be proposed.
    pub propose: bool,
    /// The envelope's height ceiling when the held proposal may be
    /// approved.
    pub approve: Option<f64>,
    /// A proposal waits, but no height bounds its approval.
    pub approve_needs_height: bool,
    /// An approved proposal may start.
    pub start: bool,
    /// The run may hold.
    pub hold: bool,
    /// The held run may resume.
    pub resume: bool,
    /// The plan may be aborted.
    pub abort: bool,
    /// A change to the plan may be drafted.
    pub revise: bool,
}

/// One client's view of one mission lane. The default desk has no name;
/// a client that sends intents builds its desk with [`MissionDesk::new`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MissionDesk {
    subject: String,
    prefix: String,
    next: u64,
    outstanding: Vec<(String, IntentKind)>,
    status: Option<Box<wire::MissionStatus>>,
    receipt: Option<FiledReceipt>,
    proposal: Option<Box<wire::MissionProposal>>,
}

impl MissionDesk {
    /// A desk for the client `subject`, started at `started_ms`; the start
    /// keeps this run's request ids apart from an earlier run's.
    #[must_use]
    pub fn new(subject: &str, started_ms: u64) -> Self {
        Self {
            subject: subject.to_owned(),
            prefix: format!("{subject}-{started_ms}"),
            next: 0,
            outstanding: Vec::new(),
            status: None,
            receipt: None,
            proposal: None,
        }
    }

    /// The client's name, which its objectives carry.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The lane's latest status; none while the host publishes none.
    #[must_use]
    pub fn status(&self) -> Option<&wire::MissionStatus> {
        self.status.as_deref()
    }

    /// The answer to this client's latest intent.
    #[must_use]
    pub fn receipt(&self) -> Option<&FiledReceipt> {
        self.receipt.as_ref()
    }

    /// The lane's state, unspecified until the lane publishes one.
    #[must_use]
    pub fn state(&self) -> Lane {
        self.status
            .as_ref()
            .and_then(|status| Lane::try_from(status.state).ok())
            .unwrap_or(Lane::Unspecified)
    }

    /// The held proposal when it is the one the lane's status names, or the
    /// lane names none yet. The lane sends a proposal only to the client
    /// that asked for it.
    #[must_use]
    pub fn current_proposal(&self) -> Option<&wire::MissionProposal> {
        let proposal = self.proposal.as_deref()?;
        let named = self
            .status
            .as_ref()
            .map_or(&[][..], |status| status.proposal_digest.as_slice());
        (named.is_empty() || named == proposal.digest.as_slice()).then_some(proposal)
    }

    /// The revision the lane's status shows waiting for review.
    #[must_use]
    pub fn drafted_successor(&self) -> Option<&wire::MissionSuccessor> {
        self.status
            .as_ref()?
            .successor
            .as_ref()
            .filter(|s| s.state == wire::MissionSuccessorState::Drafted as i32)
    }

    /// The proposal an approval names: a drafted revision first, else the
    /// plan the lane proposed. It is the record this client holds when it
    /// drafted or proposed it, else one sized from the lane's status, which
    /// carries the objective's height, for a draft another client made.
    #[must_use]
    pub fn approvable(&self) -> Option<wire::MissionProposal> {
        let status = self.status.as_ref()?;
        let digest = match self.drafted_successor() {
            Some(successor) => successor.digest.clone(),
            None => {
                if let Some(held) = self.current_proposal() {
                    return Some(held.clone());
                }
                status.proposal_digest.clone()
            }
        };
        if digest.is_empty() {
            return None;
        }
        if let Some(held) = self.proposal.as_deref().filter(|p| p.digest == digest) {
            return Some(held.clone());
        }
        Some(wire::MissionProposal {
            digest,
            objective: Some(wire::MissionObjective {
                kind: Some(wire::mission_objective::Kind::Survey(
                    wire::SurveyObjective {
                        height_m: status.height_m,
                        ..wire::SurveyObjective::default()
                    },
                )),
                ..wire::MissionObjective::default()
            }),
            ..wire::MissionProposal::default()
        })
    }

    /// The commands the lane's state offers under `policy`, none before
    /// admission.
    #[must_use]
    pub fn offered(&self, admitted: bool, policy: &ApprovalPolicy) -> Offered {
        if !admitted {
            return Offered::default();
        }
        let state = self.state();
        let reviewing = state == Lane::Proposed || self.drafted_successor().is_some();
        let proposal = self.approvable().filter(|_| reviewing);
        let height = proposal
            .as_ref()
            .and_then(|proposal| approved_height_m(proposal, policy));
        let named = self
            .status
            .as_ref()
            .is_some_and(|status| !status.proposal_digest.is_empty());
        Offered {
            propose: matches!(
                state,
                Lane::Unspecified
                    | Lane::Idle
                    | Lane::Proposed
                    | Lane::Complete
                    | Lane::Aborted
                    | Lane::Failed
            ),
            approve: height,
            approve_needs_height: proposal.is_some() && height.is_none(),
            start: state == Lane::Admitted && named,
            hold: state == Lane::Running,
            resume: state == Lane::Held,
            abort: matches!(
                state,
                Lane::Admitted | Lane::Running | Lane::Held | Lane::Uncertain
            ),
            revise: matches!(
                state,
                Lane::Proposed | Lane::Admitted | Lane::Running | Lane::Held
            ),
        }
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
                    (Some(held), Some(named)) if held.digest == named => held,
                    // Another client proposed: only a stated ceiling bounds
                    // the approval, as this client never saw the objective.
                    (None, Some(named)) if policy.max_height_m.is_some() => wire::MissionProposal {
                        digest: named,
                        ..wire::MissionProposal::default()
                    },
                    (None, None) => return Err(DeskRefusal::NoProposal),
                    (_, Some(_)) => {
                        return Err(DeskRefusal::UnknownDigest(
                            digest.clone().unwrap_or_default(),
                        ));
                    }
                };
                approval(&proposal, policy, now_ms).ok_or(DeskRefusal::NoHeight)
            }
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

    /// The bytes that send `action` under a fresh request id, and the id.
    ///
    /// # Errors
    ///
    /// Returns [`DeskRefusal::NotAdmitted`] before the host admitted the
    /// session.
    pub fn send(
        &mut self,
        engine: &ClientEngine,
        action: wire::mission_intent::Action,
    ) -> Result<(String, Vec<ClientAction>), DeskRefusal> {
        let kind = IntentKind::of(&action);
        let next = self.next.wrapping_add(1);
        let request_id = format!("{}-{next}", self.prefix);
        let actions = engine.mission_intent(wire::MissionIntent {
            request_id: request_id.clone(),
            action: Some(action),
        });
        if actions.is_empty() {
            return Err(DeskRefusal::NotAdmitted(kind.label()));
        }
        self.next = next;
        if self.outstanding.len() >= TRACKED_INTENTS {
            self.outstanding.remove(0);
        }
        self.outstanding.push((request_id.clone(), kind));
        Ok((request_id, actions))
    }

    /// Files the lane's answer to one of this client's intents. An answer
    /// to an intent this client did not send, or no longer remembers,
    /// changes nothing and returns `None`.
    pub fn on_receipt(&mut self, receipt: wire::MissionReceipt) -> Option<&FiledReceipt> {
        let index = self
            .outstanding
            .iter()
            .position(|(id, _)| *id == receipt.request_id)?;
        let (request_id, kind) = self.outstanding.remove(index);
        let outcome = wire::MissionReceiptOutcome::try_from(receipt.outcome)
            .unwrap_or(wire::MissionReceiptOutcome::Unspecified);
        let filed = FiledReceipt {
            request_id,
            kind,
            outcome,
            reason: receipt.reason,
            lane: receipt.lane,
        };
        if matches!(kind, IntentKind::Propose | IntentKind::Revise) && filed.accepted() {
            self.proposal = receipt.proposal.map(Box::new);
        }
        self.receipt = Some(filed);
        self.receipt.as_ref()
    }

    /// Takes the lane's newest status.
    pub fn on_status(&mut self, status: Box<wire::MissionStatus>) {
        self.status = Some(status);
    }
}

impl MissionDesk {
    /// The revise intent for exactly one change, continuing the plan and
    /// run the lane's status names.
    fn revision(
        &self,
        objective: Option<&super::SurveyFile>,
        reassign: Option<&super::Reassign>,
        finish: bool,
        within_envelope: bool,
        now_ms: u64,
    ) -> Result<wire::mission_intent::Action, DeskRefusal> {
        use wire::mission_revise::Change;
        let status = self.status.as_ref().ok_or(DeskRefusal::NothingToRevise)?;
        if status.proposal_digest.is_empty() {
            return Err(DeskRefusal::NothingToRevise);
        }
        let change = match (objective, reassign, finish) {
            (Some(objective), None, false) => {
                Change::Objective(objective.objective(&self.subject, now_ms))
            }
            (None, Some(reassign), false) => Change::Reassign(wire::MissionReassign {
                from_member: reassign.from.clone(),
                to_member: reassign.to.clone(),
                fraction: reassign.fraction,
            }),
            (None, None, true) => Change::Finish(wire::MissionFinish {}),
            _ => return Err(DeskRefusal::RevisionShape),
        };
        Ok(wire::mission_intent::Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            within_envelope,
            drafted_by: None,
            change: Some(change),
        }))
    }
}

/// The bytes of a hexadecimal digest.
fn parse_hex(text: &str) -> Result<Vec<u8>, DeskRefusal> {
    let refuse = || DeskRefusal::BadDigest(text.to_owned());
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err(refuse());
    }
    (0..text.len() / 2)
        .map(|i| {
            text.get(2 * i..2 * i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(refuse)
        })
        .collect()
}
