//! Adaptation within the team's envelope: a line a member flew without
//! enough usable pictures is flown again as its own small objective, under
//! the member approval the team's envelope allows, without a new operator
//! approval. Each line is flown again at most once, and the envelope's
//! `max_refly_lines` bounds the total.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;

use super::{MemberIntent, Op, Phase, TeamLane, delegate};
use crate::allocation::line_strip;
use crate::geometry::Local;

/// Margin around a member's band and launch point, metres.
const BOUND_MARGIN_M: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReflyState {
    Proposing,
    Approving,
    Starting,
    Flying,
    Done,
    Failed,
}

/// One line flown again.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Refly {
    pub member: String,
    /// The original task, such as `line-03`.
    pub line: String,
    pub state: ReflyState,
    pub proposal: Option<wire::MissionProposal>,
    /// The member's run the re-flight is, once it started.
    pub run_id: Option<String>,
}

impl Refly {
    pub(super) fn in_progress(&self) -> bool {
        !matches!(self.state, ReflyState::Done | ReflyState::Failed)
    }
}

impl TeamLane {
    /// Starts the next re-flight for `member` when its lane completed with
    /// a line flown without evidence and the envelope allows another.
    pub(super) fn adapt(
        &mut self,
        member: &str,
        status: &wire::MissionStatus,
        context: &IntentContext,
    ) -> Vec<MemberIntent> {
        if !matches!(self.phase, Phase::Running)
            || status.state != wire::MissionLaneState::Complete as i32
        {
            return Vec::new();
        }
        if self
            .reflies
            .iter()
            .any(|r| r.member == member && r.in_progress())
        {
            return Vec::new();
        }
        let Some(envelope) = self.approval.as_ref().and_then(|a| a.envelope.clone()) else {
            return Vec::new();
        };
        let used = u32::try_from(self.reflies.len()).unwrap_or(u32::MAX);
        if !envelope.permit_revise || used >= envelope.max_refly_lines {
            return Vec::new();
        }
        let tried: Vec<&str> = self
            .reflies
            .iter()
            .filter(|r| r.member == member)
            .map(|r| r.line.as_str())
            .collect();
        let Some((line, path)) = self
            .book
            .incomplete_lines(member)
            .into_iter()
            .find(|(line, _)| !tried.contains(&line.as_str()))
        else {
            return Vec::new();
        };
        let Some(objective) = self.refly_objective(member, &line, &path, context) else {
            return Vec::new();
        };
        let index = self.reflies.len();
        self.reflies.push(Refly {
            member: member.to_owned(),
            line: line.clone(),
            state: ReflyState::Proposing,
            proposal: None,
            run_id: None,
        });
        self.detail = format!(
            "{member} {line} had too few usable pictures; flying it again inside the approved envelope"
        );
        vec![self.request(
            member,
            Op::Propose,
            Some(index),
            wire::mission_intent::Action::Propose(objective),
        )]
    }

    /// A small objective covering one line: a thin strip around it, flown
    /// along the line from the member's launch point.
    fn refly_objective(
        &self,
        member: &str,
        line: &str,
        path: &[wire::GeoPoint],
        context: &IntentContext,
    ) -> Option<wire::MissionObjective> {
        let (sweep, launch) = (self.sweep?, self.member(member)?);
        let strip = line_strip(path, sweep.spacing_m)?;
        let Some(wire::mission_objective::Kind::Survey(team_survey)) =
            self.objective.as_ref().and_then(|o| o.kind.clone())
        else {
            return None;
        };
        let local = Local::around(path);
        let (a, b) = (local.project(path.first()?), local.project(path.last()?));
        let survey = wire::SurveyObjective {
            area: strip,
            launch: Some(launch.launch),
            launch_altitude_m: launch.launch_altitude_m,
            height_m: self.height_m,
            line_bearing_deg: Some((b[1] - a[1]).atan2(b[0] - a[0]).to_degrees()),
            return_to_launch: true,
            ..team_survey
        };
        Some(wire::MissionObjective {
            objective_id: format!("{}/refly/{member}/{line}", context.fresh_id),
            origin: self.objective.as_ref().and_then(|o| o.origin.clone()),
            revision: 1,
            previous_digest: Vec::new(),
            title: format!("fly {member} {line} again"),
            kind: Some(wire::mission_objective::Kind::Survey(survey)),
        })
    }

    pub(super) fn on_refly_receipt(
        &mut self,
        index: usize,
        op: Op,
        accepted: bool,
        receipt: &wire::MissionReceipt,
        _context: &IntentContext,
    ) -> Vec<MemberIntent> {
        let Some(refly) = self.reflies.get_mut(index) else {
            return Vec::new();
        };
        if !accepted {
            refly.state = ReflyState::Failed;
            self.detail = format!(
                "{} refused to fly {} again: {}",
                refly.member, refly.line, receipt.reason
            );
            return Vec::new();
        }
        let member = refly.member.clone();
        match op {
            Op::Propose => {
                let Some(proposal) = receipt.proposal.clone() else {
                    refly.state = ReflyState::Failed;
                    return Vec::new();
                };
                refly.state = ReflyState::Approving;
                refly.proposal = Some(proposal.clone());
                let launch = self.member(&member).map(|m| m.launch);
                let Some(team) = self.approval.clone() else {
                    return Vec::new();
                };
                let band = proposal
                    .tasks
                    .iter()
                    .flat_map(|t| t.path.clone())
                    .collect::<Vec<_>>();
                let approval = delegate(&team, &proposal, &band, launch, &self.config.name);
                vec![self.request(
                    &member,
                    Op::Approve,
                    Some(index),
                    wire::mission_intent::Action::Approve(approval),
                )]
            }
            Op::Approve => {
                refly.state = ReflyState::Starting;
                let digest = refly
                    .proposal
                    .as_ref()
                    .map(|p| p.digest.clone())
                    .unwrap_or_default();
                vec![self.request(
                    &member,
                    Op::Start,
                    Some(index),
                    wire::mission_intent::Action::Start(wire::MissionStart {
                        proposal_digest: digest,
                    }),
                )]
            }
            Op::Start => {
                refly.state = ReflyState::Flying;
                Vec::new()
            }
        }
    }

    /// Follows a flying re-flight in its member's status; when it ends, the
    /// original line takes the re-flight's verdict.
    pub(super) fn refly_progress(&mut self, member: &str, status: &wire::MissionStatus) {
        let Some(refly) = self
            .reflies
            .iter_mut()
            .find(|r| r.member == member && r.state == ReflyState::Flying)
        else {
            return;
        };
        if refly.run_id.is_none()
            && !status.run_id.is_empty()
            && status.state == wire::MissionLaneState::Running as i32
        {
            refly.run_id = Some(status.run_id.clone());
        }
        if refly.run_id.as_deref() != Some(status.run_id.as_str()) {
            return;
        }
        let line_status = status
            .tasks
            .iter()
            .find(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32);
        if status.state == wire::MissionLaneState::Complete as i32 {
            refly.state = ReflyState::Done;
            if let Some(line_status) = line_status {
                self.book.apply_refly(member, &refly.line, line_status);
            }
        } else if matches!(
            wire::MissionLaneState::try_from(status.state),
            Ok(wire::MissionLaneState::Failed | wire::MissionLaneState::Aborted)
        ) {
            refly.state = ReflyState::Failed;
        }
    }
}

/// A box around `band` and `launch`, widened by a margin: the bound a
/// member's own approval admits its band, its transits and its overshoot in.
pub(super) fn widened(
    band: &[wire::GeoPoint],
    launch: Option<wire::GeoPoint>,
) -> Vec<wire::GeoPoint> {
    let mut points: Vec<wire::GeoPoint> = band.to_vec();
    points.extend(launch);
    if points.is_empty() {
        return Vec::new();
    }
    let local = Local::around(&points);
    let projected: Vec<[f64; 2]> = points.iter().map(|p| local.project(p)).collect();
    let min = |axis: usize| {
        projected
            .iter()
            .map(|p| p[axis])
            .fold(f64::INFINITY, f64::min)
            - BOUND_MARGIN_M
    };
    let max = |axis: usize| {
        projected
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max)
            + BOUND_MARGIN_M
    };
    [
        [min(0), min(1)],
        [min(0), max(1)],
        [max(0), max(1)],
        [max(0), min(1)],
    ]
    .iter()
    .map(|corner| local.unproject(*corner))
    .collect()
}
