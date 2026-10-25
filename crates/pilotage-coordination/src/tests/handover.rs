//! Work moves between two members that both report only on the giving
//! member's explicit release: until then the receiving member gets nothing,
//! a refusal leaves the work with the giving member, and work released and
//! not taken is unassigned.

use pilotage_protocol::wire;
use wire::MissionHandoverState as State;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;

/// Flies `member`'s plan to its end, every line with usable pictures, and
/// reports it.
pub(super) fn complete(fleet: &mut Fleet, member: &str) {
    let lane = fleet.members.get_mut(member).expect("member");
    let lines = lane.task_ids().len().saturating_sub(1);
    for line in 0..lines {
        lane.on_progress(&toward(lane, 2 * line + 1));
        for _ in 0..3 {
            if let Some(ticket) = lane.on_frame() {
                lane.on_sent(&ticket);
                let usable = Some(FrameAssessment {
                    usable: true,
                    hazard: false,
                });
                lane.on_assessment(&ticket, usable).unwrap();
            }
        }
    }
    lane.on_progress(&RunProgress {
        plan_complete: true,
        ..toward(lane, 2 * lines)
    });
    let status = lane.take_changed().expect("changed");
    let intents = fleet.team.on_member_status(&status, &context("done"));
    fleet.route(intents);
}

/// Asks the team, inside its envelope, to move all of SIM-1's unflown lines
/// to SIM-2; the intents the team sends.
pub(super) fn move_sim1_to_sim2(fleet: &mut Fleet) -> Vec<MemberIntent> {
    let status = fleet.team.status();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-move".to_owned(),
            action: Some(Action::Revise(wire::MissionRevise {
                base_digest: status.proposal_digest.clone(),
                run_id: status.run_id.clone(),
                within_envelope: true,
                drafted_by: None,
                release: None,
                share_of: String::new(),
                change: Some(wire::mission_revise::Change::Reassign(
                    wire::MissionReassign {
                        from_member: "SIM-1".to_owned(),
                        to_member: "SIM-2".to_owned(),
                        fraction: 1.0,
                    },
                )),
            })),
        },
        &context("move"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    reply.intents
}

pub(super) fn handover(fleet: &Fleet) -> wire::MissionHandover {
    fleet.team.status().handovers.pop().expect("a handover")
}

/// SIM-1 ends its line in progress, with usable pictures, and switches to
/// its revision; its status, not yet reported.
pub(super) fn sim1_switches(fleet: &mut Fleet) -> wire::MissionStatus {
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    for _ in 0..3 {
        if let Some(ticket) = lane.on_frame() {
            lane.on_sent(&ticket);
            let usable = Some(FrameAssessment {
                usable: true,
                hazard: false,
            });
            lane.on_assessment(&ticket, usable).unwrap();
        }
    }
    let effect = lane.on_progress(&toward(lane, 2));
    assert!(matches!(effect, Some(LaneEffect::Start(_))));
    lane.take_changed().expect("changed")
}

#[test]
fn the_receiving_member_gets_nothing_before_the_release() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    assert!(
        intents.iter().all(|i| i.member == "SIM-1"),
        "only SIM-1 hears of it: {intents:?}"
    );
    assert_eq!(handover(&fleet).state, State::Releasing as i32);
    // SIM-1's revision is delayed, and SIM-1 reports that it flies on:
    // SIM-2, idle, still gets nothing.
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    lane.on_progress(&toward(lane, 1));
    let flying = lane.status();
    let more = fleet.team.on_member_status(&flying, &context("delayed"));
    assert!(more.is_empty(), "{more:?}");
    assert_eq!(handover(&fleet).state, State::Releasing as i32);

    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let released = status.released.first().cloned().expect("a release fact");
    assert_eq!(released.assignment_id, handover(&fleet).assignment_id);
    let sent = fleet.team.on_member_status(&status, &context("released"));
    assert!(sent.iter().any(|i| i.member == "SIM-2"), "{sent:?}");
    let committed = handover(&fleet);
    assert_eq!(committed.state, State::Committed as i32);
    assert_eq!(committed.generation, 2);
}

#[test]
fn a_release_that_names_another_generation_or_revision_commits_nothing() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let forged = |edit: &dyn Fn(&mut wire::WorkReleased)| {
        let mut forged = status.clone();
        forged.released.iter_mut().for_each(edit);
        forged
    };
    for wrong in [
        forged(&|r| r.generation = 7),
        forged(&|r| r.digest = b"another revision".to_vec()),
        forged(&|r| r.superseded_run_id = "another run".to_owned()),
    ] {
        let sent = fleet.team.on_member_status(&wrong, &context("forged"));
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(handover(&fleet).state, State::Releasing as i32);
    }
    let sent = fleet.team.on_member_status(&status, &context("released"));
    assert!(sent.iter().any(|i| i.member == "SIM-2"));
}

#[test]
fn a_refused_share_withdraws_the_handover_and_the_work_stays() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    // SIM-1 stops before its share arrives, so it refuses the share.
    let sim1 = fleet.members.get_mut("SIM-1").expect("SIM-1");
    sim1.on_intent(
        &wire::MissionIntent {
            request_id: "stop".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("stop"),
    );
    fleet.route(intents);
    assert_eq!(handover(&fleet).state, State::Withdrawn as i32);
    assert_eq!(
        fleet.team.status().successor.map(|s| s.state),
        Some(wire::MissionSuccessorState::Withdrawn as i32)
    );
    let more = fleet.fly(&[]);
    assert!(more.iter().all(|i| i.member != "SIM-2"), "{more:?}");
}

#[test]
fn work_released_and_refused_by_the_receiver_is_unassigned() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let sent = fleet.team.on_member_status(&status, &context("released"));
    let to_sim2 = sent.iter().find(|i| i.member == "SIM-2").expect("SIM-2");
    let refused = wire::MissionReceipt {
        request_id: to_sim2.intent.request_id.clone(),
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        reason: "the camera failed".to_owned(),
        lane: "SIM-2".to_owned(),
        ..wire::MissionReceipt::default()
    };
    let reply = fleet
        .team
        .on_member_receipt("SIM-2", &refused, &context("refused"));
    assert!(reply.intents.is_empty());
    let unassigned = handover(&fleet);
    assert_eq!(unassigned.state, State::Unassigned as i32);
    assert!(
        unassigned.basis.contains("the camera failed"),
        "{}",
        unassigned.basis
    );
    // The work does not go back to SIM-1.
    let more = fleet.fly(&[]);
    assert!(more.is_empty(), "{more:?}");
}

#[test]
fn a_restarted_coordinator_cannot_start_work_while_a_member_flies() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    // The coordinator restarts: its team lane starts empty.
    fleet.team = team();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "again".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("again"),
    );
    let decided = fleet.route(reply.intents);
    assert!(
        decided.first().is_some_and(|r| !accepted(r)),
        "a member that flies refuses a new proposal: {decided:?}"
    );
    assert_eq!(
        fleet.members.get("SIM-1").map(Lane::state),
        Some(wire::MissionLaneState::Running)
    );
    assert_ne!(
        fleet.members.get("SIM-2").map(Lane::state),
        Some(wire::MissionLaneState::Running),
        "nothing started on the idle member"
    );
}

#[test]
fn a_moved_line_the_giver_began_first_stays_with_it() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    let moving = handover(&fleet).task_ids;
    // The revision is late: SIM-1 ends line 1 and begins line 2, which moves.
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    lane.on_progress(&toward(lane, 3));
    fleet.route(intents);
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    let effect = lane.on_progress(&toward(lane, 4));
    assert!(
        matches!(effect, Some(LaneEffect::Start(_))),
        "the switch at line 2's end"
    );
    let status = lane.take_changed().expect("changed");
    fleet.team.on_member_status(&status, &context("released"));
    let committed = handover(&fleet);
    assert_eq!(committed.state, State::Committed as i32);
    assert_eq!(
        committed.task_ids.len(),
        moving.len() - 1,
        "{}",
        committed.basis
    );
    assert!(
        committed.basis.contains("except the 1 it began first"),
        "{}",
        committed.basis
    );
}

#[test]
fn moved_work_of_a_receiver_stated_down_before_it_starts_is_unassigned() {
    let (mut fleet, _) = running_fleet(true);
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let queued = fleet.team.on_member_status(&status, &context("released"));
    assert!(queued.is_empty(), "SIM-2 flies its own lines first");
    assert_eq!(handover(&fleet).state, State::Committed as i32);
    fleet.team.on_member_lost("SIM-2");
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "down".to_owned(),
            action: Some(Action::MemberDown(wire::MissionMemberDown {
                member: "SIM-2".to_owned(),
                stated_by: None,
            })),
        },
        &context("down"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    assert_eq!(handover(&fleet).state, State::Unassigned as i32);
}

#[test]
fn a_restarted_coordinator_shows_its_journaled_handovers_as_unreconciled() {
    let (mut fleet, _) = running_fleet(true);
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    fleet.team.on_member_status(&status, &context("released"));
    // The host wrote these before it sent SIM-2 the moved lines.
    let journal = fleet.team.handovers();
    assert_eq!(journal[0].state, State::Committed as i32);

    fleet.team = team();
    fleet.team.restore_handovers(&journal);
    let shown = handover(&fleet);
    assert_eq!(shown.state, State::Unreconciled as i32);
    assert_eq!(shown.task_ids, journal[0].task_ids);
    assert!(
        shown
            .basis
            .contains("check whether SIM-1 or SIM-2 flies these lines"),
        "{}",
        shown.basis
    );
    // Nothing moves on a restored handover.
    let sim2 = fleet.members.get("SIM-2").map(Lane::status).expect("SIM-2");
    assert!(
        fleet
            .team
            .on_member_status(&sim2, &context("again"))
            .is_empty()
    );
}
