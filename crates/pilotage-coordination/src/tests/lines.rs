//! The team's line grid and its members' plans agree line for line.

use pilotage_protocol::wire;

use super::*;

#[test]
fn each_member_plans_exactly_the_lines_the_team_gave_it() {
    use wire::mission_intent::Action;
    let mut fleet = Fleet::new();
    let propose = wire::MissionIntent {
        request_id: "op-1".to_owned(),
        action: Some(Action::Propose(objective())),
    };
    let reply = fleet.team.on_intent(&propose, &context("team-proposal"));
    let decided = fleet.route(reply.intents);
    let proposal = decided
        .first()
        .and_then(|r| r.proposal.clone())
        .expect("a team proposal");
    let d = defaults();
    let Some(wire::mission_objective::Kind::Survey(survey)) = objective().kind else {
        panic!("a survey");
    };
    let sweep = crate::allocation::sweep_of(&survey, d.height_m, d.across_track_fov_deg, d.sidelap);
    let launches: Vec<(String, wire::GeoPoint)> =
        members().into_iter().map(|m| (m.name, m.launch)).collect();
    let assignments = allocate(
        &survey.area,
        &launches,
        sweep.bearing_deg,
        sweep.footprint_m,
        sweep.spacing_m,
    );
    // Each planned line sits where the team placed it, across the lines.
    let local = crate::geometry::Local::around(&survey.area);
    let bearing = sweep.bearing_deg.to_radians();
    let axis = [-bearing.sin(), bearing.cos()];
    let across = |p: &wire::GeoPoint| crate::geometry::dot(local.project(p), axis);
    let low = survey.area.iter().map(across).fold(f64::INFINITY, f64::min);
    let high = survey
        .area
        .iter()
        .map(across)
        .fold(f64::NEG_INFINITY, f64::max);
    let offsets = crate::geometry::sweep_offsets(high - low, sweep.footprint_m, sweep.spacing_m);
    for assignment in &assignments {
        let mut planned: Vec<f64> = proposal
            .tasks
            .iter()
            .filter(|t| {
                t.member == assignment.member && t.kind == wire::MissionTaskKind::SurveyLine as i32
            })
            .filter_map(|t| t.path.first().map(|p| across(p) - low))
            .collect();
        planned.sort_by(f64::total_cmp);
        assert_eq!(
            planned.len(),
            assignment.lines.len(),
            "{}",
            assignment.member
        );
        for (line, at) in assignment.lines.iter().zip(&planned) {
            let team = offsets[line - 1];
            // A band is cut 2 m inside its outer swath edges.
            assert!(
                (team - at).abs() < 2.5,
                "{} line {line}: team {team:.1} m, planned {at:.1} m",
                assignment.member
            );
        }
    }
}

/// The Princeton survey: 2.6 km of lines at a bearing, where a member's
/// tangent plane and the team's frame differ measurably.
fn princeton() -> Vec<wire::GeoPoint> {
    vec![
        point(40.331086, -74.682164),
        point(40.350486, -74.661164),
        point(40.347514, -74.656436),
        point(40.328114, -74.677436),
    ]
}

/// Lines the member's planner lays in `area`, flown from `launch`.
fn planned_lines(
    area: Vec<wire::GeoPoint>,
    launch: wire::GeoPoint,
    sidelap: f64,
    bearing: f64,
) -> usize {
    use wire::mission_intent::Action;
    let mut lane = vehicle_lane("MEMBER");
    let objective = wire::MissionObjective {
        objective_id: "strip".to_owned(),
        revision: 1,
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area,
                launch: Some(launch),
                height_m: 110.0,
                across_track_fov_deg: 69.6,
                sidelap,
                line_bearing_deg: Some(bearing),
                return_to_launch: true,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    };
    let reply = lane.on_intent(
        &wire::MissionIntent {
            request_id: "p".to_owned(),
            action: Some(Action::Propose(objective)),
        },
        &context("strip"),
    );
    reply
        .receipt
        .proposal
        .map_or(0, |p| p.estimates.map_or(0, |e| e.lines as usize))
}

#[test]
fn a_strip_of_the_teams_lines_is_planned_line_for_line_at_survey_scale() {
    use crate::allocation::{Across, sidelap_for, sweep_of, team_step};
    let area = princeton();
    let survey = wire::SurveyObjective {
        area: area.clone(),
        line_bearing_deg: Some(129.526),
        ..wire::SurveyObjective::default()
    };
    let sweep = sweep_of(&survey, 110.0, 69.6, 0.6);
    let step = team_step(&area, &sweep);
    let sidelap = sidelap_for(step, sweep.footprint_m);
    let across = Across::new(&area, sweep.bearing_deg);
    let positions: Vec<f64> = area
        .iter()
        .filter_map(|p| across.of_path(std::slice::from_ref(p)))
        .collect();
    let low = positions.iter().copied().fold(f64::INFINITY, f64::min);
    let half = sweep.footprint_m / 2.0;
    // Every launch point a member of the Princeton fleet flies from.
    for launch in [point(40.330674, -74.680471), point(40.349298, -74.660311)] {
        let total = crate::geometry::sweep_offsets(
            positions.iter().copied().fold(f64::NEG_INFINITY, f64::max) - low,
            sweep.footprint_m,
            sweep.spacing_m,
        )
        .len();
        for (first, count) in [(0, 4), (10, 5), (20, 12), (total - 9, 9)] {
            let lines: Vec<f64> = (first..first + count)
                .map(|i| low + half + i as f64 * step)
                .collect();
            let strip = across.strip(&area, &lines, sweep.footprint_m);
            let planned = planned_lines(strip, launch, sidelap, sweep.bearing_deg);
            assert_eq!(
                planned, count,
                "{count} lines from line {first}, launch {launch:?}"
            );
        }
    }
}
