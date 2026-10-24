//! Extra flights a member flies after its plan, each as its own small
//! objective under the member approval the team's envelope allows.
//!
//! A re-flight is adaptation within the envelope, decided mid-run from the
//! judge's verdicts: a line a member flew without enough usable pictures
//! is flown again, without a new operator approval, once the verdict has
//! settled (every picture sent for its judgment has an answer). A member
//! that already completed flies it at once; otherwise the line's own
//! member flies it after its plan. Each line is flown again at most once,
//! and the team spends only the part of the envelope's `max_refly_lines`
//! that it keeps for its vehicle members (`team/budget.rs`); each member
//! team spends its own part. Lines a revision moved to a member
//! wait in a queue until the member completes its own plan; they never
//! splice into a plan in progress.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;

use super::{MemberIntent, Op, Phase, TeamLane, delegate};
use crate::allocation::line_strip;
use crate::geometry::Local;

/// Margin around a member's band and launch point, metres.
const BOUND_MARGIN_M: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReflyState {
    /// Moved lines waiting for the member to complete its plan.
    Queued,
    Proposing,
    Approving,
    Starting,
    Flying,
    Done,
    Failed,
    /// The team aborted before the flight started: it never flies.
    Cancelled,
    /// The team aborted after it sent the start: the member may fly it.
    Uncertain,
}

/// One line flown again.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Refly {
    /// The member that flies it.
    pub member: String,
    /// The member whose line it is; the line takes the re-flight's verdict.
    pub owner: String,
    /// The line's path, for a re-flight.
    pub path: Vec<wire::GeoPoint>,
    /// The original task, such as `line-03`.
    pub line: String,
    pub state: ReflyState,
    pub proposal: Option<wire::MissionProposal>,
    /// The member's run the re-flight is, once it started.
    pub run_id: Option<String>,
    /// The paths of lines moved to the member; empty for a re-flight.
    pub moved: Vec<Vec<wire::GeoPoint>>,
}

impl Refly {
    pub(super) fn in_progress(&self) -> bool {
        !matches!(
            self.state,
            ReflyState::Done | ReflyState::Failed | ReflyState::Cancelled
        )
    }
}

impl TeamLane {
    /// Decides re-flights from the settled verdicts and starts the extra
    /// flights of members that are free to fly them.
    pub(super) fn adapt(&mut self, context: &IntentContext) -> Vec<MemberIntent> {
        if !matches!(self.phase, Phase::Running) {
            return Vec::new();
        }
        self.decide_reflights();
        self.dispatch(context)
    }

    /// Queues a re-flight for each settled line without enough usable
    /// pictures, while the envelope allows another.
    fn decide_reflights(&mut self) {
        let Some(envelope) = self.approval.as_ref().and_then(|a| a.envelope.clone()) else {
            return;
        };
        if !envelope.permit_revise {
            return;
        }
        // A member team decides the re-flights of its own lines.
        let members: Vec<String> = self
            .member_proposals
            .keys()
            .filter(|m| self.statuses.get(*m).is_none_or(|s| s.members.is_empty()))
            .cloned()
            .collect();
        for owner in members {
            for (line, path) in self.book.incomplete_lines(&owner) {
                let flown_again = self.reflies.iter().filter(|r| r.moved.is_empty()).count();
                if u32::try_from(flown_again).unwrap_or(u32::MAX) >= self.own_refly_budget() {
                    return;
                }
                let tried = self
                    .reflies
                    .iter()
                    .any(|r| r.owner == owner && r.line == line);
                if tried || !self.book.settled(&owner, &line) {
                    continue;
                }
                // Another member flies it, so the second look does not come
                // from the camera that failed: one that has finished now, else
                // the first to finish. The owner flies it only when no other
                // member is still flying to take it.
                let flyer = self.idle_member_besides(&owner).unwrap_or_default();
                self.detail = if flyer.is_empty() {
                    format!(
                        "judge: {owner} {line} had too few usable pictures; the next member to finish flies it again, inside the approved envelope"
                    )
                } else {
                    format!(
                        "judge: {owner} {line} had too few usable pictures; {flyer} has finished and flies it again now, inside the approved envelope"
                    )
                };
                self.reflies.push(Refly {
                    member: flyer,
                    owner: owner.clone(),
                    path,
                    line,
                    state: ReflyState::Queued,
                    proposal: None,
                    run_id: None,
                    moved: Vec::new(),
                });
            }
        }
    }

    /// The member completed its plan and has no extra flight.
    fn idle(&self, member: &str) -> bool {
        self.statuses
            .get(member)
            .is_some_and(|s| s.state == wire::MissionLaneState::Complete as i32)
            && !self
                .reflies
                .iter()
                .any(|r| r.member == member && r.in_progress())
    }

    /// A member other than `owner` that is idle.
    fn idle_member_besides(&self, owner: &str) -> Option<String> {
        self.member_proposals
            .keys()
            .find(|m| m.as_str() != owner && self.idle(m))
            .cloned()
    }

    /// Starts the first queued extra flight of each member that completed
    /// its plan and flies nothing else. A re-flight whose line has since
    /// been judged usable is dropped.
    fn dispatch(&mut self, context: &IntentContext) -> Vec<MemberIntent> {
        let mut intents = Vec::new();
        let members: Vec<String> = self.member_proposals.keys().cloned().collect();
        for member in members {
            let complete = self
                .statuses
                .get(&member)
                .is_some_and(|s| s.state == wire::MissionLaneState::Complete as i32);
            let busy = self
                .reflies
                .iter()
                .any(|r| r.member == member && r.in_progress() && r.state != ReflyState::Queued);
            if !complete || busy {
                continue;
            }
            let Some(index) = self.next_extra(&member) else {
                continue;
            };
            if let Some(extra) = self.reflies.get_mut(index) {
                extra.member.clone_from(&member);
            }
            intents.extend(self.start_extra(index, context));
        }
        intents
    }

    /// The extra flight a free `member` takes next: one assigned to it,
    /// else a re-flight of another member's line, else of its own line
    /// once no other member is still flying to take it.
    fn next_extra(&self, member: &str) -> Option<usize> {
        let queued = |r: &Refly| r.state == ReflyState::Queued;
        if let Some(index) = self
            .reflies
            .iter()
            .position(|r| queued(r) && r.member == member)
        {
            return Some(index);
        }
        let open = |r: &Refly| queued(r) && r.member.is_empty();
        if let Some(index) = self
            .reflies
            .iter()
            .position(|r| open(r) && r.owner != member)
        {
            return Some(index);
        }
        let others_flying = self.member_proposals.keys().any(|m| {
            m != member
                && !self.stated_down(m)
                && self
                    .statuses
                    .get(m)
                    .is_some_and(|s| s.state != wire::MissionLaneState::Complete as i32)
        });
        if others_flying {
            return None;
        }
        self.reflies
            .iter()
            .position(|r| open(r) && r.owner == member)
    }

    fn start_extra(&mut self, index: usize, context: &IntentContext) -> Vec<MemberIntent> {
        let Some(extra) = self.reflies.get(index).cloned() else {
            return Vec::new();
        };
        if !extra.moved.is_empty() {
            return self.start_moved(index, context);
        }
        let still_bad = self
            .book
            .incomplete_lines(&extra.owner)
            .iter()
            .any(|(line, _)| *line == extra.line);
        let objective = still_bad
            .then(|| self.refly_objective(&extra.member, &extra.line, &extra.path, context))
            .flatten();
        let Some(objective) = objective else {
            if let Some(entry) = self.reflies.get_mut(index) {
                entry.state = ReflyState::Done;
            }
            return Vec::new();
        };
        if let Some(entry) = self.reflies.get_mut(index) {
            entry.state = ReflyState::Proposing;
        }
        vec![self.request(
            &extra.member,
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
        let strip = line_strip(path, self.sweep?.spacing_m)?;
        let local = Local::around(path);
        let (a, b) = (local.project(path.first()?), local.project(path.last()?));
        let mut objective = self.member_objective(
            member,
            strip,
            format!("{}/refly/{member}/{line}", context.fresh_id),
            format!("fly {member} {line} again"),
        )?;
        if let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind.as_mut() {
            survey.line_bearing_deg = Some((b[1] - a[1]).atan2(b[0] - a[0]).to_degrees());
        }
        Some(objective)
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
        // A receipt takes the next step only of a step still waiting for it.
        if !matches!(
            refly.state,
            ReflyState::Proposing | ReflyState::Approving | ReflyState::Starting
        ) {
            return Vec::new();
        }
        if !accepted {
            refly.state = ReflyState::Failed;
            self.detail = format!(
                "{} refused to fly {} again: {}",
                refly.member, refly.line, receipt.reason
            );
            if !refly.moved.is_empty() {
                let (member, owner) = (refly.member.clone(), refly.owner.clone());
                self.unassign_moved(&member, &owner, &receipt.reason);
            }
            return Vec::new();
        }
        let member = refly.member.clone();
        match op {
            Op::Propose => {
                let Ok(proposal) = super::opened(receipt).map(|s| s.proposal().clone()) else {
                    refly.state = ReflyState::Failed;
                    return Vec::new();
                };
                refly.state = ReflyState::Approving;
                refly.proposal = Some(proposal.clone());
                self.approve_extra(index, &member, &proposal)
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
            Op::Revise | Op::Renew => Vec::new(),
        }
    }

    /// The approval of the extra flight `index`, proposed by `member` as
    /// `proposal`, passed down inside the team's own approval.
    fn approve_extra(
        &mut self,
        index: usize,
        member: &str,
        proposal: &wire::MissionProposal,
    ) -> Vec<MemberIntent> {
        let launch = self.member(member).map(|m| m.launch);
        let Some(team) = self.approval.clone() else {
            return Vec::new();
        };
        let band: Vec<wire::GeoPoint> =
            proposal.tasks.iter().flat_map(|t| t.path.clone()).collect();
        let approval = delegate(
            &team,
            &proposal.proposal_id,
            &proposal.digest,
            &band,
            launch,
            &self.config.name,
        );
        // The team pays for this flight: the member decides no re-flight
        // inside it, so it spends no part twice.
        let approval = super::budget::without_budget(approval);
        vec![self.request(
            member,
            Op::Approve,
            Some(index),
            wire::mission_intent::Action::Approve(approval),
        )]
    }

    /// Follows a flying re-flight in its member's status; when it ends, the
    /// original line takes the re-flight's verdict.
    pub(super) fn refly_progress(&mut self, member: &str, status: &wire::MissionStatus) {
        let Some(refly) = self.reflies.iter_mut().find(|r| {
            r.member == member && matches!(r.state, ReflyState::Flying | ReflyState::Uncertain)
        }) else {
            return;
        };
        if refly.run_id.is_none()
            && !status.run_id.is_empty()
            && status.state == wire::MissionLaneState::Running as i32
        {
            refly.run_id = Some(status.run_id.clone());
            if !refly.moved.is_empty() {
                // Moved lines are new work: the book follows their run.
                self.book.adopt(member, &status.run_id);
                self.book.record(status, &[]);
            }
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
            if let (Some(line_status), true) = (line_status, refly.moved.is_empty()) {
                self.book
                    .apply_refly(&refly.owner, &refly.line, line_status);
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
