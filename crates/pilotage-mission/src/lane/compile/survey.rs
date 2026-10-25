//! The survey kind: an area flown in parallel lines at one height, each
//! line a task whose evidence is its usable pictures, then the way home.

use navigate_contract::GeodeticPosition;
use pilotage_protocol::wire;

use super::{Compiled, distance_m, seal, task_id};
use crate::engine::MissionEngine;
use crate::lane::{LaneConfig, RunPlan};
use crate::survey::{SurveyPlan, SurveyRequest, plan_survey};

/// The values an objective's zero fields take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurveyDefaults {
    /// Height above the launch point, metres.
    pub height_m: f64,
    /// Along-track cruise speed, metres per second.
    pub cruise_mps: f64,
    /// Across-track camera field of view, degrees.
    pub across_track_fov_deg: f64,
    /// Overlap between neighbouring lines, a fraction.
    pub sidelap: f64,
}

impl Default for SurveyDefaults {
    fn default() -> Self {
        Self {
            height_m: 110.0,
            cruise_mps: 2.0,
            across_track_fov_deg: 69.6,
            sidelap: 0.6,
        }
    }
}

/// Compiles the survey `survey` of `objective` under `lane`'s configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    survey: &wire::SurveyObjective,
    lane: &LaneConfig,
    proposal_id: String,
) -> Result<Compiled, String> {
    let launch = survey
        .launch
        .as_ref()
        .ok_or("the survey objective has no launch point")?;
    if survey.area.len() < 3 {
        return Err(format!(
            "the survey area has {} vertices; it needs at least 3",
            survey.area.len()
        ));
    }
    let defaults = lane.defaults;
    let height_m = pick(survey.height_m, defaults.height_m);
    let cruise_mps = pick(survey.cruise_mps, defaults.cruise_mps);
    let request = survey_request(survey, &defaults, cruise_mps);
    let mut config = lane.template.clone();
    config.anchor = GeodeticPosition::new(
        launch.latitude_deg.to_radians(),
        launch.longitude_deg.to_radians(),
        survey.launch_altitude_m,
    );
    config.cruise_height_m = height_m;
    config.cruise_mps = cruise_mps;
    config.limits.max_horizontal_mps = config.limits.max_horizontal_mps.max(cruise_mps);
    let plan = plan_survey(&request, &config.anchor, height_m).map_err(|e| e.to_string())?;
    let (_, record) =
        MissionEngine::from_survey(&request, config.clone()).map_err(|e| e.to_string())?;
    let tasks = tasks_of(&plan, request.return_to_launch, &proposal_id);
    config.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
    let duration_s = record.path_length_m / cruise_mps + height_m / config.climb_rate_mps.max(0.1);
    let estimates = wire::MissionEstimates {
        path_length_m: record.path_length_m,
        duration_s,
        energy_wh: lane
            .cruise_power_w
            .map_or(0.0, |watts| watts * duration_s / 3600.0),
        lines: u32::try_from(record.line_count).unwrap_or(u32::MAX),
        line_spacing_m: record.line_spacing_m,
        footprint_width_m: record.footprint_width_m,
    };
    let assumptions = wire::MissionAssumptions {
        execution_strategy: lane.execution_strategy.clone(),
        vehicle_name: lane.name.clone(),
        simulated: lane.simulated,
        max_horizontal_mps: config.limits.max_horizontal_mps,
        plan_digest: record.mission_identity.content_digest.as_bytes().to_vec(),
        plan_revision: record.mission_identity.revision_id.clone(),
        usable_energy_wh: lane.usable_energy_wh.unwrap_or(0.0),
    };
    let proposal = wire::MissionProposal {
        proposal_id,
        objective_id: objective.objective_id.clone(),
        digest: Vec::new(),
        tasks,
        estimates: Some(estimates),
        assumptions: Some(assumptions),
        members: Vec::new(),
        objective: Some(objective.clone()),
        predecessor_digest: Vec::new(),
        summary: String::new(),
        requires: Vec::new(),
    };
    let (proposal, record) = seal(proposal);
    Ok(Compiled {
        proposal,
        record,
        plan: RunPlan::Survey(request),
        config,
        height_m,
        min_usable_per_line: survey.min_usable_frames_per_line.max(1),
    })
}

fn pick(value: f64, default: f64) -> f64 {
    if value > 0.0 { value } else { default }
}

/// The planner's request for `survey`, its zero fields at `defaults`.
fn survey_request(
    survey: &wire::SurveyObjective,
    defaults: &SurveyDefaults,
    cruise_mps: f64,
) -> SurveyRequest {
    let mut request = SurveyRequest::new(
        survey
            .area
            .iter()
            .map(|p| [p.latitude_deg, p.longitude_deg])
            .collect(),
    );
    request.across_track_fov_rad =
        pick(survey.across_track_fov_deg, defaults.across_track_fov_deg).to_radians();
    request.sidelap = if survey.sidelap > 0.0 {
        survey.sidelap
    } else {
        defaults.sidelap
    };
    request.return_to_launch = survey.return_to_launch;
    request.line_bearing_rad = survey.line_bearing_deg.map(f64::to_radians);
    request.fit_to_cruise(cruise_mps);
    request
}

/// One task per survey line, then the way home.
fn tasks_of(plan: &SurveyPlan, return_home: bool, proposal_id: &str) -> Vec<wire::MissionTaskPlan> {
    let point = |index: usize| {
        plan.plan
            .waypoints
            .get(index)
            .map(|waypoint| wire::GeoPoint {
                latitude_deg: waypoint.position.latitude_rad.to_degrees(),
                longitude_deg: waypoint.position.longitude_rad.to_degrees(),
            })
    };
    let mut tasks: Vec<wire::MissionTaskPlan> = (0..plan.line_count)
        .filter_map(|line| {
            let path = vec![point(2 * line)?, point(2 * line + 1)?];
            Some(wire::MissionTaskPlan {
                task_id: task_id(proposal_id, &line.to_string()),
                name: format!("line-{:02}", line + 1),
                kind: wire::MissionTaskKind::SurveyLine as i32,
                length_m: distance_m(&path[0], &path[1]),
                path,
                member: String::new(),
                operation: None,
            })
        })
        .collect();
    if return_home
        && let (Some(from), Some(home)) =
            (point(2 * plan.line_count - 1), point(2 * plan.line_count))
    {
        tasks.push(wire::MissionTaskPlan {
            task_id: task_id(proposal_id, "home"),
            name: "home".to_owned(),
            kind: wire::MissionTaskKind::ReturnHome as i32,
            length_m: distance_m(&from, &home),
            path: vec![from, home],
            member: String::new(),
            operation: None,
        });
    }
    tasks
}
