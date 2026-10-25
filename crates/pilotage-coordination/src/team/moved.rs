//! Lines a revision moved to a member: they wait in the member's queue until
//! it completes its own plan, then fly as a small objective of their own.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;

use super::refly::{Refly, ReflyState};
use super::{MemberIntent, Op, TeamLane};
use crate::allocation::Across;

impl TeamLane {
    /// Queues lines a revision moved to `member`; they start at once when
    /// the member already completed its plan.
    pub(super) fn queue_moved(
        &mut self,
        member: &str,
        from: &str,
        moved: Vec<Vec<wire::GeoPoint>>,
        assignment: String,
        context: &IntentContext,
    ) -> Vec<MemberIntent> {
        let index = self.reflies.len();
        self.reflies.push(Refly {
            member: member.to_owned(),
            owner: from.to_owned(),
            path: Vec::new(),
            line: format!("{} lines from {from}", moved.len()),
            state: ReflyState::Queued,
            proposal: None,
            run_id: None,
            moved,
            assignment: Some(assignment),
            prior_run: None,
        });
        let complete = self
            .statuses
            .get(member)
            .is_some_and(|s| s.state == wire::MissionLaneState::Complete as i32);
        if complete {
            self.start_moved(index, context)
        } else {
            Vec::new()
        }
    }

    /// Proposes the moved lines of `index` to its member: the team's area
    /// clipped to those lines' swaths.
    pub(super) fn start_moved(
        &mut self,
        index: usize,
        context: &IntentContext,
    ) -> Vec<MemberIntent> {
        let Some(extra) = self.reflies.get(index).cloned() else {
            return Vec::new();
        };
        let (Some(sweep), Some(area)) = (
            self.sweep,
            self.objective.as_ref().and_then(|o| {
                o.kind
                    .as_ref()
                    .map(|wire::mission_objective::Kind::Survey(s)| s.area.clone())
            }),
        ) else {
            return Vec::new();
        };
        let across = Across::new(&area, sweep.bearing_deg);
        let positions: Vec<f64> = extra
            .moved
            .iter()
            .filter_map(|p| across.of_path(p))
            .collect();
        let strip = across.strip(&area, &positions, sweep.footprint_m);
        let Some(objective) = self.member_objective(
            &extra.member,
            strip,
            format!("{}/moved/{}/{index}", context.fresh_id, extra.member),
            format!("{}: {}", extra.member, extra.line),
        ) else {
            return Vec::new();
        };
        if let Some(entry) = self.reflies.get_mut(index) {
            entry.state = ReflyState::Proposing;
        }
        self.detail = format!("{} flies {} after its own lines", extra.member, extra.line);
        vec![self.request(
            &extra.member,
            Op::Propose,
            Some(index),
            wire::mission_intent::Action::Propose(objective),
        )]
    }
}
