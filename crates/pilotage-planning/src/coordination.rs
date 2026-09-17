//! Local coordinated mission drafts. A draft grants no vehicle authority.

#[cfg(test)]
mod tests;
mod timing;
mod validation;

use crate::{PlanningError, RoutePlan};
use serde::{Deserialize, Serialize};
pub use timing::{CoordinationSummary, TimingAssessment, assess_coordination};

/// A named planning participant. Identity verification occurs outside this document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionParticipant {
    /// Local participant identity.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Organization name, when supplied.
    pub organization: Option<String>,
}

/// Planned work for one vehicle under one participant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    /// Stable assignment identity.
    pub id: String,
    /// Assignment display name.
    pub name: String,
    /// Participant responsible for the assignment.
    pub participant_id: String,
    /// Durable vehicle reference. A live session must bind it separately.
    pub vehicle_reference: Option<String>,
    /// The resolved route for this assignment.
    pub route: RoutePlan,
}

/// Assignments that coordinate as one swarm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Swarm {
    /// Stable group identity.
    pub id: String,
    /// Group display name.
    pub name: String,
    /// Exact member assignment identities.
    pub assignment_ids: Vec<String>,
}

/// The route occurrence where a member must meet a timing target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingMember {
    /// The affected assignment.
    pub assignment_id: String,
    /// Exact occurrence in that assignment's route.
    pub waypoint_id: String,
    /// Member time relative to the common target, in seconds.
    pub offset_seconds: i32,
}

/// A time-on-target condition for one assignment or a complete swarm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingTarget {
    /// Stable timing condition identity.
    pub id: String,
    /// Target display name.
    pub name: String,
    /// Common time on target in UTC Unix seconds.
    pub utc: i64,
    /// Permitted early and late deviation, in seconds.
    pub tolerance_seconds: u32,
    /// Swarm identity when this condition applies to a swarm.
    pub swarm_id: Option<String>,
    /// Member route occurrences and time offsets.
    pub members: Vec<TimingMember>,
}

/// One editable coordinated mission. Publishing does not execute it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinatedPlan {
    /// This implementation accepts schema version 1.
    pub schema_version: u32,
    /// Stable mission identity.
    pub id: String,
    /// Local revision counter. It is not a server conflict token.
    pub revision: u64,
    /// Mission display name.
    pub title: String,
    /// Participants named in this draft.
    pub participants: Vec<MissionParticipant>,
    /// Separate vehicle assignments.
    pub assignments: Vec<Assignment>,
    /// Swarm membership fixed for this revision.
    pub swarms: Vec<Swarm>,
    /// Individual and swarm time-on-target conditions.
    pub timing_targets: Vec<TimingTarget>,
}

/// Encodes a reviewable revision and its SHA-256 digest. No authority is granted.
pub fn export_revision(plan: &CoordinatedPlan) -> Result<String, PlanningError> {
    use sha2::{Digest, Sha256};
    plan.validate()?;
    let bytes = serde_json::to_vec(plan)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(serde_json::to_string(
        &serde_json::json!({"schema_version":1,
        "plan":plan, "content_digest":digest, "assessment":assess_coordination(plan)?}),
    )?)
}
