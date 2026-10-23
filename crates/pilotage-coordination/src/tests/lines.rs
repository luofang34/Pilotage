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
            assert!(
                (team - at).abs() < 1.0,
                "{} line {line}: team {team:.1} m, planned {at:.1} m",
                assignment.member
            );
        }
    }
}
