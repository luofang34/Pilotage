//! Compiles an objective into a grounded proposal: the plan the host runs,
//! its tasks, estimates and assumptions. Each kind of objective plans in its
//! own module; what follows the plan, its approval, run and evidence, is the
//! same for every kind.

mod operations;
mod report;
mod survey;

pub use survey::SurveyDefaults;

use pilotage_protocol::wire;

use super::{LaneConfig, RunPlan};
use crate::config::MissionConfig;
use pilotage_protocol::proposal_record::{
    FEATURE_OPERATION, FEATURE_RETURN_HOME, FEATURE_SURVEY_LINE, SealedProposal,
};

/// A compiled proposal and what executes it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Compiled {
    /// The decoded proposal; its digest is the record's.
    pub proposal: wire::MissionProposal,
    /// The proposal as it travels: its bytes are encoded once, here.
    pub record: wire::MissionProposalRecord,
    pub plan: RunPlan,
    pub config: MissionConfig,
    pub height_m: f64,
    pub min_usable_per_line: u32,
}

impl Compiled {
    /// The plan is a flight. A plan of operations has no way home to fly,
    /// and is not revised: an abort ends it, and a new proposal replaces it.
    pub(super) fn flies(&self) -> bool {
        !matches!(self.plan, RunPlan::Operations(_))
    }
}

/// Compiles `objective` under `lane`'s configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    lane: &LaneConfig,
    proposal_id: String,
) -> Result<Compiled, String> {
    match objective.kind.as_ref() {
        Some(wire::mission_objective::Kind::Survey(survey)) => {
            survey::compile(objective, survey, lane, proposal_id)
        }
        Some(wire::mission_objective::Kind::Report(report)) => {
            report::compile(objective, report, lane, proposal_id)
        }
        None => Err("the objective names no kind".to_owned()),
    }
}

/// A plan that ends the work of `base` and flies home from wherever the
/// vehicle is, at the same height and speed.
pub(super) fn home(base: &Compiled, proposal_id: String) -> Compiled {
    let anchor = &base.config.anchor;
    let home = wire::GeoPoint {
        latitude_deg: anchor.latitude_rad.to_degrees(),
        longitude_deg: anchor.longitude_rad.to_degrees(),
    };
    let home_id = task_id(&proposal_id, "home");
    let task_ids = vec![home_id.clone()];
    let proposal = wire::MissionProposal {
        proposal_id,
        objective_id: base.proposal.objective_id.clone(),
        digest: Vec::new(),
        tasks: vec![wire::MissionTaskPlan {
            task_id: home_id,
            kind: wire::MissionTaskKind::ReturnHome as i32,
            path: vec![home],
            length_m: 0.0,
            member: String::new(),
            name: "home".to_owned(),
            operation: None,
        }],
        estimates: Some(wire::MissionEstimates::default()),
        assumptions: base.proposal.assumptions.clone(),
        members: Vec::new(),
        objective: base.proposal.objective.clone(),
        predecessor_digest: Vec::new(),
        summary: String::new(),
        requires: Vec::new(),
    };
    let (proposal, record) = seal(proposal);
    Compiled {
        proposal,
        record,
        plan: RunPlan::Home,
        config: MissionConfig {
            task_ids,
            ..base.config.clone()
        },
        height_m: base.height_m,
        min_usable_per_line: base.min_usable_per_line,
    }
}

/// Makes `compiled` the successor of `predecessor`: its digest binds the predecessor and
/// the summary. Its tasks keep their identities, unique to its own proposal.
pub(super) fn succeed(compiled: &mut Compiled, predecessor: &[u8], summary: String) {
    let mut proposal = compiled.proposal.clone();
    proposal.predecessor_digest = predecessor.to_vec();
    proposal.summary = summary;
    (compiled.proposal, compiled.record) = seal(proposal);
}

/// Encodes `proposal` once into its record, with the features its tasks
/// need.
pub(super) fn seal(
    mut proposal: wire::MissionProposal,
) -> (wire::MissionProposal, wire::MissionProposalRecord) {
    let mut requires = Vec::new();
    for (kind, feature) in [
        (wire::MissionTaskKind::SurveyLine, FEATURE_SURVEY_LINE),
        (wire::MissionTaskKind::ReturnHome, FEATURE_RETURN_HOME),
        (wire::MissionTaskKind::Operation, FEATURE_OPERATION),
    ] {
        if proposal.tasks.iter().any(|t| t.kind == kind as i32) {
            requires.push(feature.to_owned());
        }
    }
    proposal.requires = requires;
    SealedProposal::seal(proposal).into_parts()
}

/// The identity of the task `key` of proposal `proposal_id`: opaque, unique to the
/// proposal. Nothing reads it back for its parts.
pub(super) fn task_id(proposal_id: &str, key: &str) -> String {
    format!("{proposal_id}#{key}")
}

/// Great-circle distance on a sphere of the WGS-84 equatorial radius.
pub(super) fn distance_m(a: &wire::GeoPoint, b: &wire::GeoPoint) -> f64 {
    const RADIUS_M: f64 = 6_378_137.0;
    let (lat_a, lat_b) = (a.latitude_deg.to_radians(), b.latitude_deg.to_radians());
    let d_lat = lat_b - lat_a;
    let d_lon = (b.longitude_deg - a.longitude_deg).to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat_a.cos() * lat_b.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * RADIUS_M * h.sqrt().min(1.0).asin()
}
