//! A team that is a member of another team: the parent follows the lines its member team
//! flew again.

use pilotage_protocol::wire;

use super::*;

/// Delivers a parent team's intents to the fleet's team, and the team's
/// receipts and status back to the parent, until the exchange settles.
/// Returns the operator receipts the parent decided.
fn relay(
    parent: &mut TeamLane,
    fleet: &mut Fleet,
    mut intents: Vec<MemberIntent>,
) -> Vec<wire::MissionReceipt> {
    let mut decided = Vec::new();
    while let Some(MemberIntent { intent, .. }) = intents.pop() {
        let ctx = context("relay");
        let reply = fleet.team.on_intent(&intent, &ctx);
        let routed = fleet.route(reply.intents);
        for receipt in reply.receipt.into_iter().chain(routed) {
            let answer = parent.on_member_receipt("TEAM", &receipt, &ctx);
            decided.extend(answer.receipt);
            intents.extend(answer.intents);
        }
        if let Some(status) = fleet.team.take_changed() {
            intents.extend(parent.on_member_status(&status, &ctx));
        }
    }
    decided
}

#[test]
fn a_parent_team_counts_the_lines_its_member_team_flew_again() {
    use wire::mission_intent::Action;
    let mut fleet = Fleet::new();
    let mut parent = TeamLane::new(TeamConfig {
        name: "COMMAND".to_owned(),
        members: vec![Member {
            name: "TEAM".to_owned(),
            launch: point(40.3328, -74.6722),
            launch_altitude_m: 0.0,
        }],
        defaults: defaults(),
        recovery_policy: "hold".to_owned(),
    });
    let intent = |id: &str, action| wire::MissionIntent {
        request_id: id.to_owned(),
        action: Some(action),
    };
    let reply = parent.on_intent(&intent("p", Action::Propose(objective())), &context("p"));
    let proposal = relay(&mut parent, &mut fleet, reply.intents)
        .first()
        .and_then(opened)
        .expect("the parent proposal");
    let approve = Action::Approve(approval(&proposal.digest, 2));
    let reply = parent.on_intent(&intent("a", approve), &context("a"));
    assert!(
        relay(&mut parent, &mut fleet, reply.intents)
            .first()
            .is_some_and(accepted)
    );
    let start = Action::Start(wire::MissionStart {
        proposal_digest: proposal.digest.clone(),
    });
    let reply = parent.on_intent(&intent("s", start), &context("s"));
    assert!(
        relay(&mut parent, &mut fleet, reply.intents)
            .first()
            .is_some_and(accepted)
    );
    for bad in [&[("SIM-2", "line-01")][..], &[]] {
        let refly = fleet.fly(bad);
        fleet.route(refly);
        if let Some(status) = fleet.team.take_changed() {
            parent.on_member_status(&status, &context("fly"));
        }
    }
    let status = parent.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
    let line = status
        .tasks
        .iter()
        .find(|t| t.name == "TEAM/SIM-2/line-01")
        .expect("the nested line");
    assert_eq!(line.reflown, 1);
    assert!(status.detail.contains("1 flown again"), "{}", status.detail);
}

/// Delivers a parent's intents to its members, the fleet's team `TEAM` and
/// the vehicle lane `vehicle`, and their answers back, until it settles.
fn relay_two(
    parent: &mut TeamLane,
    fleet: &mut Fleet,
    vehicle: &mut Lane,
    intents: Vec<MemberIntent>,
) -> Vec<wire::MissionReceipt> {
    relay_recording(parent, fleet, vehicle, intents, &mut Vec::new())
}

/// [`relay_two`], keeping every intent the parent sent in `sent`.
fn relay_recording(
    parent: &mut TeamLane,
    fleet: &mut Fleet,
    vehicle: &mut Lane,
    mut intents: Vec<MemberIntent>,
    sent: &mut Vec<MemberIntent>,
) -> Vec<wire::MissionReceipt> {
    let mut decided = Vec::new();
    while let Some(MemberIntent { member, intent }) = intents.pop() {
        sent.push(MemberIntent {
            member: member.clone(),
            intent: intent.clone(),
        });
        let ctx = context("relay");
        let receipts = if member == "TEAM" {
            let reply = fleet.team.on_intent(&intent, &ctx);
            let routed = fleet.route(reply.intents);
            reply.receipt.into_iter().chain(routed).collect()
        } else {
            vec![vehicle.on_intent(&intent, &ctx).receipt]
        };
        for receipt in receipts {
            let answer = parent.on_member_receipt(&member, &receipt, &ctx);
            decided.extend(answer.receipt);
            intents.extend(answer.intents);
        }
        let statuses: Vec<wire::MissionStatus> = fleet
            .team
            .take_changed()
            .into_iter()
            .chain(vehicle.take_changed())
            .collect();
        for status in statuses {
            intents.extend(parent.on_member_status(&status, &ctx));
        }
    }
    decided
}

/// A parent over the fleet's team `TEAM` and the vehicle `SIM-9`, whose
/// approval allows three re-flights inside a bound around the area, started.
fn started_parent(fleet: &mut Fleet, vehicle: &mut Lane) -> TeamLane {
    use wire::mission_intent::Action;
    let mut parent = TeamLane::new(TeamConfig {
        name: "COMMAND".to_owned(),
        members: vec![
            Member {
                name: "TEAM".to_owned(),
                launch: point(40.3328, -74.6722),
                launch_altitude_m: 0.0,
            },
            Member {
                name: "SIM-9".to_owned(),
                launch: point(40.3368, -74.6648),
                launch_altitude_m: 0.0,
            },
        ],
        defaults: defaults(),
        recovery_policy: "hold".to_owned(),
    });
    let intent = |id: &str, action| wire::MissionIntent {
        request_id: id.to_owned(),
        action: Some(action),
    };
    let reply = parent.on_intent(&intent("p", Action::Propose(objective())), &context("p"));
    let proposal = relay_two(&mut parent, fleet, vehicle, reply.intents)
        .first()
        .and_then(opened)
        .expect("the parent proposal");
    let mut approved = approval(&proposal.digest, 3);
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.bound = approved_bound();
    }
    let reply = parent.on_intent(&intent("a", Action::Approve(approved)), &context("a"));
    let decided = relay_two(&mut parent, fleet, vehicle, reply.intents);
    assert!(decided.first().is_some_and(accepted), "{decided:?}");
    let start = Action::Start(wire::MissionStart {
        proposal_digest: proposal.digest.clone(),
    });
    let reply = parent.on_intent(&intent("s", start), &context("s"));
    relay_two(&mut parent, fleet, vehicle, reply.intents);
    parent
}

#[test]
fn a_member_team_spends_only_its_part_of_the_re_flight_budget() {
    let mut fleet = Fleet::new();
    let mut vehicle = vehicle_lane("SIM-9");
    // Three lines in all: the team gets two, the parent keeps one for SIM-9.
    let _parent = started_parent(&mut fleet, &mut vehicle);
    let bad = [
        ("SIM-1", "line-01"),
        ("SIM-2", "line-01"),
        ("SIM-3", "line-01"),
    ];
    for _ in 0..4 {
        let refly = fleet.fly(&bad);
        fleet.route(refly);
    }
    let reflown: u32 = fleet.team.status().tasks.iter().map(|t| t.reflown).sum();
    assert_eq!(reflown, 2, "{}", fleet.team.status().detail);
}

#[test]
fn a_member_team_flying_its_parents_re_flight_gets_no_budget_for_it() {
    use wire::mission_intent::Action;
    let intent = |id: &str, action| wire::MissionIntent {
        request_id: id.to_owned(),
        action: Some(action),
    };
    let mut fleet = Fleet::new();
    let mut vehicle = vehicle_lane("SIM-9");
    let mut parent = started_parent(&mut fleet, &mut vehicle);
    // TEAM finishes first, with every line usable.
    let refly = fleet.fly(&[]);
    fleet.route(refly);
    if let Some(status) = fleet.team.take_changed() {
        let intents = parent.on_member_status(&status, &context("team-done"));
        relay_two(&mut parent, &mut fleet, &mut vehicle, intents);
    }
    // SIM-9 flies its lines; its first line has no usable picture.
    let lines = vehicle.task_ids().len().saturating_sub(1);
    for line in 0..lines {
        vehicle.on_progress(&toward(&vehicle, 2 * line + 1));
        for _ in 0..3 {
            if let Some(ticket) = vehicle.on_frame() {
                vehicle.on_sent(&ticket);
                let usable = Some(FrameAssessment {
                    usable: line != 0,
                    hazard: false,
                });
                vehicle.on_assessment(&ticket, usable).unwrap();
            }
        }
    }
    vehicle.on_progress(&RunProgress {
        plan_complete: true,
        ..toward(&vehicle, 2 * lines)
    });
    let status = vehicle.take_changed().expect("SIM-9 reports");
    let intents = parent.on_member_status(&status, &context("sim9-done"));
    let mut sent = Vec::new();
    relay_recording(&mut parent, &mut fleet, &mut vehicle, intents, &mut sent);
    let budgets: Vec<u32> = sent
        .iter()
        .filter(|i| i.member == "TEAM")
        .filter_map(|i| match &i.intent.action {
            Some(Action::Approve(a)) => a.envelope.as_ref().map(|e| e.max_refly_lines),
            _ => None,
        })
        .collect();
    assert_eq!(
        budgets,
        [0],
        "TEAM flies the parent's re-flight with no budget"
    );
    // A renewal while TEAM flies that extra flight gives it no budget either.
    let status = parent.status();
    let mut renewal = approval(&status.proposal_digest, 3);
    renewal.run_id.clone_from(&status.run_id);
    if let Some(envelope) = renewal.envelope.as_mut() {
        envelope.bound = approved_bound();
    }
    let reply = parent.on_intent(&intent("r", Action::Approve(renewal)), &context("r"));
    let renewed: Vec<u32> = reply
        .intents
        .iter()
        .filter(|i| i.member == "TEAM")
        .filter_map(|i| match &i.intent.action {
            Some(Action::Approve(a)) => a.envelope.as_ref().map(|e| e.max_refly_lines),
            _ => None,
        })
        .collect();
    assert_eq!(renewed, [0], "{}", parent.status().detail);
}

/// A bound around the area and every launch point.
fn approved_bound() -> Vec<wire::GeoPoint> {
    vec![
        point(40.3300, -74.6760),
        point(40.3300, -74.6610),
        point(40.3400, -74.6610),
        point(40.3400, -74.6760),
    ]
}
