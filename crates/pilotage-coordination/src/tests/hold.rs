//! A team hold: the team reports that a hold was asked until every member
//! that still flies reports that it holds in place.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;

#[test]
fn a_team_holds_only_once_every_member_holds() {
    let (mut fleet, _) = running_fleet(false);
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    fleet.route(reply.intents);
    let state = |fleet: &Fleet| fleet.team.status().state;
    assert_eq!(state(&fleet), wire::MissionLaneState::HoldRequested as i32);
    let names: Vec<String> = fleet.members.keys().cloned().collect();
    for (index, name) in names.iter().enumerate() {
        let lane = fleet.members.get_mut(name).expect("member");
        lane.on_progress(&RunProgress {
            target_waypoint: Some(1),
            holding: true,
            ..RunProgress::default()
        });
        if let Some(status) = lane.take_changed() {
            fleet.team.on_member_status(&status, &context("held"));
        }
        let expected = if index + 1 == names.len() {
            wire::MissionLaneState::Held
        } else {
            wire::MissionLaneState::HoldRequested
        };
        assert_eq!(state(&fleet), expected as i32, "after {name}");
    }
}
