//! A team revision inside the envelope applies only when the envelope
//! permits every class of change it makes, by the same check a vehicle lane
//! uses.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;

fn revise_within(fleet: &mut Fleet, change: wire::mission_revise::Change) -> Vec<MemberIntent> {
    let status = fleet.team.status();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-revise".to_owned(),
            action: Some(Action::Revise(wire::MissionRevise {
                base_digest: status.proposal_digest.clone(),
                run_id: status.run_id.clone(),
                within_envelope: true,
                drafted_by: None,
                change: Some(change),
            })),
        },
        &context("revision"),
    );
    let receipt = reply.receipt.expect("answered");
    assert!(accepted(&receipt), "{}", receipt.reason);
    reply.intents
}

#[test]
fn a_reassignment_needs_the_allocate_permission() {
    let (mut fleet, _) = running_fleet(false);
    let intents = revise_within(
        &mut fleet,
        wire::mission_revise::Change::Reassign(wire::MissionReassign {
            from_member: "SIM-3".to_owned(),
            to_member: "SIM-1".to_owned(),
            fraction: 1.0,
        }),
    );
    assert!(intents.is_empty(), "nothing applies");
    let detail = fleet.team.status().detail;
    assert!(detail.contains("permit_allocate"), "{detail}");
}

#[test]
fn a_smaller_team_area_needs_the_objective_permission() {
    let (mut fleet, _) = running_fleet(true);
    let mut revised = objective();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = revised.kind.as_mut() {
        survey.height_m = defaults().height_m;
        let first = survey.area[0];
        for point in &mut survey.area {
            point.latitude_deg =
                first.latitude_deg + (point.latitude_deg - first.latitude_deg) / 2.0;
        }
    }
    assert_eq!(revised.objective_id, objective().objective_id);
    let intents = revise_within(&mut fleet, wire::mission_revise::Change::Objective(revised));
    assert!(intents.is_empty(), "nothing applies");
    let detail = fleet.team.status().detail;
    assert!(
        detail.contains("permit_change_objective for the area"),
        "{detail}"
    );
}
