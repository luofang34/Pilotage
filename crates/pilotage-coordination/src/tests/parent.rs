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
        .and_then(|r| r.proposal.clone())
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
