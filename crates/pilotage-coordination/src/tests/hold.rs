//! A team hold: the team reports that a hold was asked until every member
//! that still flies reports that it holds in place. A team that waits for
//! its results takes a hold too, and flies no line again until resumed.

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
            holding: true,
            execution: None,
            ..toward(lane, 1)
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

#[test]
fn a_parent_holds_while_its_member_team_waits_for_its_results() {
    use super::parent::{relay_recording, started_parent};
    let mut fleet = Fleet::new();
    let mut vehicle = vehicle_lane("SIM-9");
    let mut parent = started_parent(&mut fleet, &mut vehicle);
    // The member team flies its plan to the end, and one line's pictures
    // wait for a judgment.
    let (_, waiting) = fleet.fly_holding(&[], Some(("SIM-2", "line-01")));
    assert!(!waiting.is_empty());
    let team = fleet.team.take_changed().expect("the team changed");
    assert_eq!(
        team.execution.as_ref().map(|e| e.state),
        Some(wire::MissionExecutionState::AwaitingResults as i32)
    );
    parent.on_member_status(&team, &context("settling"));

    let reply = parent.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    relay_recording(
        &mut parent,
        &mut fleet,
        &mut vehicle,
        reply.intents,
        &mut Vec::new(),
    );
    assert_eq!(
        parent.status().state,
        wire::MissionLaneState::HoldRequested as i32,
        "SIM-9 has not held yet"
    );
    vehicle.on_progress(&RunProgress {
        holding: true,
        execution: None,
        ..toward(&vehicle, 1)
    });
    let held = vehicle.take_changed().expect("SIM-9 changed");
    parent.on_member_status(&held, &context("held"));
    let status = parent.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Held as i32,
        "{}",
        status.detail
    );
}

#[test]
fn a_held_team_flies_no_line_again_until_it_is_resumed() {
    let (mut fleet, _) = running_fleet(false);
    let (_, waiting) = fleet.fly_holding(&[], Some(("SIM-2", "line-01")));
    assert!(
        !waiting.is_empty(),
        "the line's pictures wait for a judgment"
    );
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    assert!(
        reply.receipt.is_some_and(|r| accepted(&r)),
        "a settling team holds"
    );
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Held as i32
    );

    // The late judgment finds the line unusable while the team holds.
    let mut intents = Vec::new();
    for (member, ticket) in waiting {
        let lane = fleet.members.get_mut(&member).expect("member");
        lane.on_assessment(
            &ticket,
            Some(FrameAssessment {
                usable: false,
                hazard: false,
            }),
        )
        .expect("taken");
        if let Some(status) = lane.take_changed() {
            intents.extend(fleet.team.on_member_status(&status, &context("judged")));
        }
    }
    assert!(
        intents.is_empty(),
        "no line flies again while held: {intents:?}"
    );
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Held as i32
    );

    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-resume".to_owned(),
            action: Some(Action::Resume(wire::MissionResume {})),
        },
        &context("resume"),
    );
    assert!(
        reply
            .intents
            .iter()
            .any(|i| matches!(i.intent.action, Some(Action::Propose(_)))),
        "the line flies again once resumed: {:?}",
        reply.intents
    );
}

#[test]
fn a_line_flown_again_that_a_hold_overtakes_starts_only_once_resumed() {
    let (mut fleet, _) = running_fleet(false);
    let refly = fleet.fly(&[("SIM-1", "line-01")]);
    let flier = refly
        .iter()
        .find(|i| matches!(i.intent.action, Some(Action::Propose(_))))
        .map(|i| i.member.clone())
        .expect("a line flies again");
    // The hold comes while the extra flight is proposed.
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    fleet.route(reply.intents);
    fleet.route(refly);
    let state = |fleet: &Fleet| fleet.members.get(&flier).map(Lane::state);
    assert_eq!(
        state(&fleet),
        Some(wire::MissionLaneState::Admitted),
        "approved, not started: {}",
        fleet.team.status().detail
    );

    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-resume".to_owned(),
            action: Some(Action::Resume(wire::MissionResume {})),
        },
        &context("resume"),
    );
    fleet.route(reply.intents);
    assert_eq!(state(&fleet), Some(wire::MissionLaneState::Running));
}

#[test]
fn a_member_whose_start_a_hold_overtook_is_held_too() {
    let (mut fleet, _) = running_fleet(false);
    // The extra flight goes until its start, which is not yet delivered.
    let mut intents = fleet.fly(&[("SIM-1", "line-01")]);
    let start = loop {
        let MemberIntent { member, intent } = intents.pop().expect("a start");
        if matches!(intent.action, Some(Action::Start(_))) {
            break MemberIntent { member, intent };
        }
        let lane = fleet.members.get_mut(&member).expect("member");
        let reply = lane.on_intent(&intent, &context("step"));
        intents.extend(
            fleet
                .team
                .on_member_receipt(&member, &reply.receipt, &context("step"))
                .intents,
        );
    };
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    fleet.route(reply.intents);
    // The start lands after the hold: the team holds that member too.
    fleet.route(vec![start.clone()]);
    assert_eq!(
        fleet.members.get(&start.member).map(Lane::state),
        Some(wire::MissionLaneState::HoldRequested)
    );
}

#[test]
fn a_member_stated_down_flies_none_of_its_approved_work() {
    use super::revision::member_down;
    let (mut fleet, _) = running_fleet(false);
    let refly = fleet.fly(&[("SIM-1", "line-01")]);
    let flier = refly
        .iter()
        .find(|i| matches!(i.intent.action, Some(Action::Propose(_))))
        .map(|i| i.member.clone())
        .expect("a line flies again");
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-hold".to_owned(),
            action: Some(Action::Hold(wire::MissionHold {})),
        },
        &context("hold"),
    );
    fleet.route(reply.intents);
    fleet.route(refly);
    fleet.team.on_member_lost(&flier);
    let mut down = member_down(&flier);
    if let Action::MemberDown(statement) = &mut down {
        statement.member.clone_from(&flier);
    }
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-down".to_owned(),
            action: Some(down),
        },
        &context("down"),
    );
    assert!(reply.receipt.is_none_or(|r| accepted(&r)));
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-resume".to_owned(),
            action: Some(Action::Resume(wire::MissionResume {})),
        },
        &context("resume"),
    );
    assert!(
        !reply
            .intents
            .iter()
            .any(|i| i.member == flier && matches!(i.intent.action, Some(Action::Start(_)))),
        "{:?}",
        reply.intents
    );
}
