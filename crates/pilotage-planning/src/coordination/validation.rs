//! Referential checks for coordinated mission drafts.

use super::{CoordinatedPlan, TimingTarget};
use crate::{PlanningError, error::invalid, navigation::text};
use std::collections::BTreeSet;

impl CoordinatedPlan {
    /// Checks references and timing contracts without granting execution authority.
    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.schema_version != 1 {
            return Err(invalid("schema_version", "expected 1"));
        }
        text("mission.id", &self.id)?;
        text("mission.title", &self.title)?;
        unique(
            "participants",
            self.participants.iter().map(|v| v.id.as_str()),
        )?;
        unique(
            "assignments",
            self.assignments.iter().map(|v| v.id.as_str()),
        )?;
        unique("swarms", self.swarms.iter().map(|v| v.id.as_str()))?;
        unique(
            "timing_targets",
            self.timing_targets.iter().map(|v| v.id.as_str()),
        )?;
        for participant in &self.participants {
            text(&participant.id, &participant.name)?;
            if let Some(org) = &participant.organization {
                text(&participant.id, org)?;
            }
        }
        for assignment in &self.assignments {
            text(&assignment.id, &assignment.name)?;
            if !self
                .participants
                .iter()
                .any(|p| p.id == assignment.participant_id)
            {
                return Err(invalid(&assignment.id, "participant is absent"));
            }
            if let Some(vehicle) = &assignment.vehicle_reference {
                text(&assignment.id, vehicle)?;
            }
            assignment.route.validate()?;
        }
        for swarm in &self.swarms {
            text(&swarm.id, &swarm.name)?;
            unique(&swarm.id, swarm.assignment_ids.iter().map(String::as_str))?;
            if swarm.assignment_ids.is_empty()
                || swarm
                    .assignment_ids
                    .iter()
                    .any(|id| !self.assignments.iter().any(|a| a.id == *id))
            {
                return Err(invalid(
                    &swarm.id,
                    "swarm members must name existing assignments",
                ));
            }
        }
        for target in &self.timing_targets {
            self.validate_target(target)?;
        }
        Ok(())
    }

    fn validate_target(&self, target: &TimingTarget) -> Result<(), PlanningError> {
        text(&target.id, &target.name)?;
        if !(0..=253_402_300_799).contains(&target.utc) || target.tolerance_seconds > 86400 {
            return Err(invalid(
                &target.id,
                "target time or tolerance is outside planning limits",
            ));
        }
        unique(
            &target.id,
            target.members.iter().map(|m| m.assignment_id.as_str()),
        )?;
        if target.members.is_empty() {
            return Err(invalid(&target.id, "target needs at least one member"));
        }
        for member in &target.members {
            let assignment = self
                .assignments
                .iter()
                .find(|a| a.id == member.assignment_id)
                .ok_or_else(|| invalid(&target.id, "target assignment is absent"))?;
            if !assignment
                .route
                .waypoints
                .iter()
                .any(|w| w.id == member.waypoint_id)
            {
                return Err(invalid(
                    &target.id,
                    "target waypoint is absent; remove its time condition before deleting it",
                ));
            }
            if member.offset_seconds.unsigned_abs() > 86400 {
                return Err(invalid(&target.id, "member offset exceeds one day"));
            }
        }
        if let Some(id) = &target.swarm_id {
            let swarm = self
                .swarms
                .iter()
                .find(|s| s.id == *id)
                .ok_or_else(|| invalid(id, "swarm is absent"))?;
            let expected: BTreeSet<_> = swarm.assignment_ids.iter().collect();
            let actual: BTreeSet<_> = target.members.iter().map(|m| &m.assignment_id).collect();
            if expected != actual {
                return Err(invalid(
                    &target.id,
                    "timing members must equal swarm membership",
                ));
            }
        } else if target.members.len() != 1 {
            return Err(invalid(&target.id, "multiple members require a swarm"));
        }
        Ok(())
    }
}

fn unique<'a>(field: &str, ids: impl Iterator<Item = &'a str>) -> Result<(), PlanningError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        text(field, id)?;
        if !seen.insert(id) {
            return Err(invalid(field, "duplicate identity"));
        }
        if seen.len() > 256 {
            return Err(invalid(field, "more than 256 records"));
        }
    }
    Ok(())
}
