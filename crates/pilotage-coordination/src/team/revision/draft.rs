//! Drafting a team revision: which lines move or stay, and the proposal
//! a reviewer reads.

use pilotage_mission::IntentContext;
use pilotage_protocol::proposal_record::SealedProposal;
use pilotage_protocol::wire;

use super::Change;
use crate::allocation::{Across, team_step};
use crate::team::TeamLane;

impl TeamLane {
    pub(super) fn draft_reassign(
        &self,
        reassign: &wire::MissionReassign,
    ) -> Result<(Change, String), String> {
        let (from, to) = (reassign.from_member.as_str(), reassign.to_member.as_str());
        for name in [from, to] {
            if self.member(name).is_none() {
                return Err(format!("{name} is not a member of {}", self.config.name));
            }
        }
        if from == to {
            return Err("a reassignment moves lines between two members".to_owned());
        }
        if !(reassign.fraction > 0.0 && reassign.fraction <= 1.0) {
            return Err(format!(
                "the fraction {} is not in (0, 1]",
                reassign.fraction
            ));
        }
        let across = self.across().ok_or("the team has no sweep")?;
        // A lost member may still fly: its work stays reserved until it
        // reports again or a person states that it is down. A member that is
        // down cannot finish its line in progress either.
        let lines = match self.lost.get(from) {
            Some(wire::MemberLink::Lost) => {
                return Err(format!(
                    "{from}'s link is lost and its state is unknown; its work stays reserved until it reports again or a person states that it is down"
                ));
            }
            Some(_) => self.book.unfinished_lines(from),
            None => self.book.unflown_lines(from),
        };
        let mut unflown: Vec<(f64, Vec<wire::GeoPoint>)> = lines
            .into_iter()
            .filter_map(|(_, path)| Some((across.of_path(&path)?, path)))
            .collect();
        if unflown.is_empty() {
            return Err(format!("{from} has no unflown lines to move"));
        }
        let mean = |paths: &[Vec<wire::GeoPoint>]| {
            let positions: Vec<f64> = paths.iter().filter_map(|p| across.of_path(p)).collect();
            positions.iter().sum::<f64>() / positions.len().max(1) as f64
        };
        let toward_high = mean(&self.book.line_paths(to)) > mean(&self.book.line_paths(from));
        unflown.sort_by(|a, b| a.0.total_cmp(&b.0));
        if toward_high {
            unflown.reverse();
        }
        let count = unflown.len();
        let moving = ((count as f64) * reassign.fraction).ceil() as usize;
        let moving = moving.clamp(1, count);
        let moved: Vec<Vec<wire::GeoPoint>> = unflown
            .iter()
            .take(moving)
            .map(|(_, p)| p.clone())
            .collect();
        let kept: Vec<Vec<wire::GeoPoint>> = unflown
            .iter()
            .skip(moving)
            .map(|(_, p)| p.clone())
            .collect();
        let summary = format!(
            "move {moving} of {from}'s {count} unflown lines to {to}; {to} flies them after its own lines{}",
            if kept.is_empty() {
                format!(", and {from} returns after its line in progress")
            } else {
                String::new()
            }
        );
        Ok((
            Change::Reassign {
                from: from.to_owned(),
                to: to.to_owned(),
                kept,
                moved,
            },
            summary,
        ))
    }

    pub(super) fn draft_keep_none(&self) -> (Change, String) {
        let keep = self
            .member_proposals
            .keys()
            .filter(|m| self.running(m))
            .map(|m| (m.clone(), Vec::new()))
            .collect();
        (
            Change::Keep(keep),
            "end every member after its line in progress; each returns to its launch point"
                .to_owned(),
        )
    }

    /// A strip of the team's own area, as a parent team sends it: each
    /// member keeps only its unflown lines inside the strip.
    pub(super) fn draft_keep(
        &self,
        objective: &wire::MissionObjective,
    ) -> Result<(Change, String), String> {
        let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind.as_ref() else {
            return Err("the revised objective is not a survey".to_owned());
        };
        let sweep = self.sweep.ok_or("the team has no sweep")?;
        let bearing = survey.line_bearing_deg.unwrap_or(sweep.bearing_deg);
        if (bearing - sweep.bearing_deg).abs() > 0.5
            || (survey.height_m - self.height_m).abs() > 0.5
        {
            return Err("a running team keeps its height and line bearing; propose a new objective for other ones".to_owned());
        }
        let across = self.across().ok_or("the team has no sweep")?;
        let area = self.area().ok_or("the team has no area")?;
        let step = team_step(&area, &sweep);
        let edges: Vec<f64> = survey
            .area
            .iter()
            .map(|p| across.of_path(std::slice::from_ref(p)).unwrap_or(f64::NAN))
            .collect();
        // The strip's edges are swath edges, half a footprint outside its
        // outer lines; a line belongs to it when it lies within half a
        // spacing of those lines, never the neighbour beyond.
        let half = sweep.footprint_m / 2.0;
        let low = edges.iter().copied().fold(f64::INFINITY, f64::min) + half - step / 2.0;
        let high = edges.iter().copied().fold(f64::NEG_INFINITY, f64::max) - half + step / 2.0;
        let mut keep = Vec::new();
        let mut dropped = 0_usize;
        for member in self.member_proposals.keys().filter(|m| self.running(m)) {
            let unflown = self.book.unflown_lines(member);
            let kept: Vec<Vec<wire::GeoPoint>> = unflown
                .iter()
                .filter(|(_, path)| {
                    across
                        .of_path(path)
                        .is_some_and(|x| (low..=high).contains(&x))
                })
                .map(|(_, path)| path.clone())
                .collect();
            if kept.len() < unflown.len() {
                dropped = dropped.saturating_add(unflown.len() - kept.len());
                keep.push((member.clone(), kept));
            }
        }
        Ok((
            Change::Keep(keep),
            format!(
                "keep the lines inside the revised area; {dropped} unflown lines leave the team"
            ),
        ))
    }

    pub(super) fn running(&self, member: &str) -> bool {
        self.statuses.get(member).is_some_and(|s| {
            matches!(
                wire::MissionLaneState::try_from(s.state),
                Ok(wire::MissionLaneState::Running
                    | wire::MissionLaneState::HoldRequested
                    | wire::MissionLaneState::Held)
            )
        })
    }

    pub(super) fn area(&self) -> Option<Vec<wire::GeoPoint>> {
        match self.objective.as_ref()?.kind.as_ref()? {
            wire::mission_objective::Kind::Survey(s) => Some(s.area.clone()),
        }
    }

    pub(super) fn across(&self) -> Option<Across> {
        let area = match self.objective.as_ref()?.kind.as_ref()? {
            wire::mission_objective::Kind::Survey(s) => s.area.clone(),
        };
        Some(Across::new(&area, self.sweep?.bearing_deg))
    }

    /// The draft for review: the team's unflown lines after the change,
    /// each with its member, bound to the plan it succeeds.
    pub(super) fn revision_proposal(
        &self,
        change: &Change,
        summary: String,
        context: &IntentContext,
    ) -> SealedProposal {
        let mut tasks = Vec::new();
        let line = |member: &str, index: usize, path: &Vec<wire::GeoPoint>| wire::MissionTaskPlan {
            task_id: format!("{}#{member}-{index}", context.fresh_id),
            name: format!("{member}/unflown-{:02}", index + 1),
            kind: wire::MissionTaskKind::SurveyLine as i32,
            path: path.clone(),
            length_m: 0.0,
            member: member.to_owned(),
        };
        for member in self.member_proposals.keys() {
            let paths: Vec<Vec<wire::GeoPoint>> = match change {
                Change::Reassign {
                    from,
                    to,
                    kept,
                    moved,
                } if member == from => kept.clone(),
                Change::Reassign { to, moved, .. } if member == to => self
                    .book
                    .unflown_lines(member)
                    .into_iter()
                    .map(|(_, p)| p)
                    .chain(moved.iter().cloned())
                    .collect(),
                Change::Keep(keep) => keep.iter().find(|(m, _)| m == member).map_or_else(
                    || {
                        self.book
                            .unflown_lines(member)
                            .into_iter()
                            .map(|(_, p)| p)
                            .collect()
                    },
                    |(_, k)| k.clone(),
                ),
                _ => self
                    .book
                    .unflown_lines(member)
                    .into_iter()
                    .map(|(_, p)| p)
                    .collect(),
            };
            tasks.extend(paths.iter().enumerate().map(|(i, p)| line(member, i, p)));
        }
        let proposal = wire::MissionProposal {
            proposal_id: context.fresh_id.clone(),
            objective_id: self
                .objective
                .as_ref()
                .map(|o| o.objective_id.clone())
                .unwrap_or_default(),
            estimates: Some(wire::MissionEstimates {
                lines: u32::try_from(tasks.len()).unwrap_or(u32::MAX),
                ..wire::MissionEstimates::default()
            }),
            tasks,
            objective: self.objective.clone(),
            predecessor_digest: self
                .proposal
                .as_ref()
                .map(|p| p.digest.clone())
                .unwrap_or_default(),
            summary,
            ..wire::MissionProposal::default()
        };
        SealedProposal::seal(proposal)
    }
}
