//! The operator's propose, approve, start and steering intents: the team
//! decomposes, delegates and fans out to its members.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;

use super::{MemberIntent, Op, Phase, TeamLane, refly};
use crate::allocation::{allocate, sweep_of};

impl TeamLane {
    pub(super) fn propose(
        &mut self,
        objective: &wire::MissionObjective,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if !matches!(
            self.phase,
            Phase::Idle | Phase::Proposed | Phase::Complete | Phase::Aborted | Phase::Failed
        ) {
            return Err(format!(
                "the team is {:?}; a new objective needs a successor plan",
                self.phase
            ));
        }
        let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind.as_ref() else {
            return Err("the objective is not a survey".to_owned());
        };
        if survey.area.len() < 3 {
            return Err("the survey area needs at least three vertices".to_owned());
        }
        let defaults = self.config.defaults;
        let pick = |value: f64, default: f64| if value > 0.0 { value } else { default };
        self.height_m = pick(survey.height_m, defaults.height_m);
        let fov = pick(survey.across_track_fov_deg, defaults.across_track_fov_deg);
        let sidelap = pick(survey.sidelap, defaults.sidelap);
        let sweep = sweep_of(survey, self.height_m, fov, sidelap);
        let launches: Vec<(String, wire::GeoPoint)> = self
            .config
            .members
            .iter()
            .map(|m| (m.name.clone(), m.launch))
            .collect();
        let assignments = allocate(
            &survey.area,
            &launches,
            sweep.bearing_deg,
            sweep.footprint_m,
            sweep.spacing_m,
        );
        if assignments.is_empty() {
            return Err("no member could take a band of the area".to_owned());
        }
        self.reset_plan();
        let mut intents = Vec::new();
        for assignment in &assignments {
            let Some(member) = self.member(&assignment.member) else {
                continue;
            };
            let mut member_survey = survey.clone();
            member_survey.area.clone_from(&assignment.band);
            member_survey.launch = Some(member.launch);
            member_survey.launch_altitude_m = member.launch_altitude_m;
            member_survey.height_m = self.height_m;
            member_survey.across_track_fov_deg = fov;
            member_survey.sidelap = sidelap;
            member_survey.line_bearing_deg = Some(sweep.bearing_deg);
            member_survey.return_to_launch = true;
            let member_objective = wire::MissionObjective {
                objective_id: format!("{}/{}", context.fresh_id, member.name),
                origin: objective.origin.clone(),
                revision: 1,
                previous_digest: Vec::new(),
                title: format!("{}: lines {:?}", objective.title, assignment.lines),
                kind: Some(wire::mission_objective::Kind::Survey(member_survey)),
            };
            intents.push(self.request(
                &assignment.member,
                Op::Propose,
                None,
                wire::mission_intent::Action::Propose(member_objective),
            ));
        }
        self.objective = Some(objective.clone());
        self.sweep = Some(sweep);
        self.assignments = assignments;
        self.phase = Phase::Proposing;
        self.detail = format!("waiting for {} member proposals", intents.len());
        Ok((intents, true))
    }

    pub(super) fn approve(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if self.phase != Phase::Proposed {
            return Err(format!(
                "an approval needs a team proposal; the team is {:?}",
                self.phase
            ));
        }
        let proposal = self.proposal.as_ref().ok_or("no team proposal")?;
        if approval.proposal_digest != proposal.digest {
            return Err("the approval binds another proposal digest".to_owned());
        }
        let envelope = valid_envelope(approval, context.now_unix_ms)?;
        if self.height_m > envelope.max_height_m + 1e-6 {
            return Err(format!(
                "the team flies at {:.1} m; the envelope allows {:.1} m",
                self.height_m, envelope.max_height_m
            ));
        }
        let mut intents = Vec::new();
        for assignment in self.assignments.clone() {
            let Some(member_proposal) = self.member_proposals.get(&assignment.member) else {
                continue;
            };
            let launch = self.member(&assignment.member).map(|m| m.launch);
            let member_approval = delegate(
                approval,
                member_proposal,
                &assignment.band,
                launch,
                &self.config.name,
            );
            intents.push(self.request(
                &assignment.member,
                Op::Approve,
                None,
                wire::mission_intent::Action::Approve(member_approval),
            ));
        }
        self.approval = Some(approval.clone());
        self.phase = Phase::Approving;
        self.detail = "passing the approval to the members".to_owned();
        Ok((intents, true))
    }

    pub(super) fn start(
        &mut self,
        start: &wire::MissionStart,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if self.phase != Phase::Admitted {
            return Err(format!(
                "a start needs an approved team proposal; the team is {:?}",
                self.phase
            ));
        }
        if self.proposal.as_ref().map(|p| &p.digest) != Some(&start.proposal_digest) {
            return Err("the start names another proposal than the approved one".to_owned());
        }
        let mut intents = Vec::new();
        for (member, proposal) in self.member_proposals.clone() {
            let action = wire::mission_intent::Action::Start(wire::MissionStart {
                proposal_digest: proposal.digest.clone(),
            });
            intents.push(self.request(&member, Op::Start, None, action));
        }
        self.phase = Phase::Starting;
        self.detail = "starting the members".to_owned();
        Ok((intents, true))
    }

    pub(super) fn steer(
        &mut self,
        from: &[Phase],
        to: Phase,
        action: wire::mission_intent::Action,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        if !from.contains(&self.phase) {
            return Err(format!("not possible while the team is {:?}", self.phase));
        }
        let members: Vec<String> = self.member_proposals.keys().cloned().collect();
        let intents = members
            .iter()
            .map(|member| {
                self.requests = self.requests.wrapping_add(1);
                MemberIntent {
                    member: member.clone(),
                    intent: wire::MissionIntent {
                        request_id: format!("{}-steer-{}", self.config.name, self.requests),
                        action: Some(action.clone()),
                    },
                }
            })
            .collect();
        self.phase = to;
        self.detail = format!("{to:?} sent to the members").to_lowercase();
        Ok((intents, false))
    }
}

/// The approval's envelope when it permits execution and has not expired.
pub(super) fn valid_envelope(
    approval: &wire::MissionApproval,
    now_unix_ms: u64,
) -> Result<&wire::AdaptationEnvelope, String> {
    let envelope = approval
        .envelope
        .as_ref()
        .ok_or("the approval has no adaptation envelope")?;
    if !envelope.permit_execute {
        return Err("the approval does not permit execution".to_owned());
    }
    if envelope.expires_unix_ms <= now_unix_ms {
        return Err("the approval has expired".to_owned());
    }
    Ok(envelope)
}

/// A member's approval inside the team's: the same limits, narrowed to the
/// member's band, bound to the member's own proposal.
pub(super) fn delegate(
    team: &wire::MissionApproval,
    member: &wire::MissionProposal,
    band: &[wire::GeoPoint],
    launch: Option<wire::GeoPoint>,
    issuer: &str,
) -> wire::MissionApproval {
    let mut envelope = team.envelope.clone().unwrap_or_default();
    // An operator's bound stays the bound; without one, the member flies
    // its band, the way to and from it, and the lines' overshoot.
    if envelope.bound.is_empty() {
        envelope.bound = refly::widened(band, launch);
    }
    envelope.permit_allocate = false;
    envelope.permit_change_objective = false;
    wire::MissionApproval {
        approval_id: format!("{}/{}", team.approval_id, member.proposal_id),
        proposal_digest: member.digest.clone(),
        approver: Some(wire::RecordOrigin {
            issuer: "pilotage-coordinator".to_owned(),
            subject: issuer.to_owned(),
            organization: team
                .approver
                .as_ref()
                .map(|a| a.organization.clone())
                .unwrap_or_default(),
        }),
        envelope: Some(envelope),
        approved_unix_ms: team.approved_unix_ms,
    }
}
