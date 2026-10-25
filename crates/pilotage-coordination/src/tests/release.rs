//! A team that is a member of another team fulfils the release contract
//! of a leaf or refuses it: it does not build a release fact from its
//! members' own, so it refuses a revision that asks for a release before it
//! changes anything, and its parent waits for nothing.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::parent::{relay_recording, started_parent};
use super::*;

#[test]
fn a_member_team_refuses_a_release_it_cannot_report_and_nothing_moves() {
    let intent = |id: &str, action| wire::MissionIntent {
        request_id: id.to_owned(),
        action: Some(action),
    };
    let mut fleet = Fleet::new();
    let mut vehicle = vehicle_lane("SIM-9");
    let mut parent = started_parent(&mut fleet, &mut vehicle);
    let status = parent.status();
    let reassign = Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        change: Some(wire::mission_revise::Change::Reassign(
            wire::MissionReassign {
                from_member: "TEAM".to_owned(),
                to_member: "SIM-9".to_owned(),
                fraction: 0.5,
            },
        )),
        ..wire::MissionRevise::default()
    });
    let drafted = parent.on_intent(&intent("r", reassign), &context("r"));
    let draft = drafted.receipt.as_ref().and_then(opened).expect("a draft");
    let applied = parent.on_intent(
        &intent("a", Action::Approve(approval(&draft.digest, 0))),
        &context("a"),
    );
    assert!(applied.receipt.is_some_and(|r| accepted(&r)));
    let asks_release = applied.intents.iter().any(|i| {
        i.member == "TEAM"
            && matches!(&i.intent.action, Some(Action::Revise(r)) if r.release.is_some())
    });
    assert!(asks_release, "the parent asks TEAM to release the lines");

    let team_before = fleet.team.status();
    let members_before: Vec<_> = fleet.members.values().map(Lane::status).collect();
    let mut sent = Vec::new();
    relay_recording(
        &mut parent,
        &mut fleet,
        &mut vehicle,
        applied.intents,
        &mut sent,
    );

    let team = fleet.team.status();
    assert_eq!(team.successor, team_before.successor, "TEAM drafts nothing");
    assert_eq!(team.tasks, team_before.tasks);
    assert_eq!(team.state, team_before.state);
    let members: Vec<_> = fleet.members.values().map(Lane::status).collect();
    assert_eq!(members, members_before, "no member of TEAM hears of it");
    let handover = parent
        .status()
        .handovers
        .into_iter()
        .next()
        .expect("a handover");
    assert_eq!(
        handover.state,
        wire::MissionHandoverState::Withdrawn as i32,
        "{}",
        handover.basis
    );
    assert!(handover.basis.contains("release"), "{}", handover.basis);
    assert!(
        sent.iter().all(|i| i.member != "SIM-9"),
        "the receiver gets nothing"
    );
}
