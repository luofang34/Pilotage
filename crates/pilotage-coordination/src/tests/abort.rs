//! A team abort ends the work the team was still setting in motion: a late
//! receipt or a late release fact is recorded, but it never starts work
//! the abort ended.

use pilotage_protocol::wire;
use wire::MissionHandoverState as State;
use wire::mission_intent::Action;

use super::handover::{complete, handover, move_sim1_to_sim2, sim1_switches};
use super::revision::running_fleet;
use super::*;

fn abort(fleet: &mut Fleet) -> Vec<MemberIntent> {
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-abort".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("abort"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    reply.intents
}

/// Delivers `intent` to its member lane and returns the member's receipt,
/// without telling the team.
fn deliver(fleet: &mut Fleet, intent: &MemberIntent) -> wire::MissionReceipt {
    let lane = fleet.members.get_mut(&intent.member).expect("member");
    lane.on_intent(&intent.intent, &context("deliver")).receipt
}

/// SIM-2 is idle, SIM-1's lines are moving to it, and SIM-1 released them:
/// the team's first intent to SIM-2 for the moved lines.
fn released_to_idle_sim2(fleet: &mut Fleet) -> MemberIntent {
    complete(fleet, "SIM-2");
    let intents = move_sim1_to_sim2(fleet);
    fleet.route(intents);
    let status = sim1_switches(fleet);
    let sent = fleet.team.on_member_status(&status, &context("released"));
    sent.into_iter()
        .find(|i| i.member == "SIM-2")
        .expect("the moved lines go to SIM-2")
}

#[test]
fn a_release_that_arrives_after_an_abort_starts_nothing() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    // The release is late: the team aborts first.
    abort(&mut fleet);
    let sent = fleet.team.on_member_status(&status, &context("late"));
    assert!(sent.iter().all(|i| i.member != "SIM-2"), "{sent:?}");
    let recorded = handover(&fleet);
    assert_eq!(
        recorded.state,
        State::Unassigned as i32,
        "{}",
        recorded.basis
    );
    // The same release again changes nothing.
    assert!(
        fleet
            .team
            .on_member_status(&status, &context("again"))
            .is_empty()
    );
}

#[test]
fn a_late_receipt_after_an_abort_takes_no_next_step() {
    for step in ["propose", "approve", "start"] {
        let (mut fleet, _) = running_fleet(true);
        let mut next = released_to_idle_sim2(&mut fleet);
        // Walk the moved lines' chain up to `step`.
        let mut receipt = deliver(&mut fleet, &next);
        for reached in ["propose", "approve"] {
            if step == reached {
                break;
            }
            let reply = fleet
                .team
                .on_member_receipt("SIM-2", &receipt, &context("r"));
            next = reply.intents.into_iter().next().expect("the next step");
            receipt = deliver(&mut fleet, &next);
        }
        abort(&mut fleet);
        let reply = fleet
            .team
            .on_member_receipt("SIM-2", &receipt, &context("late"));
        assert!(reply.intents.is_empty(), "{step}: {:?}", reply.intents);
        let state = handover(&fleet).state;
        if step == "start" {
            assert_eq!(
                state,
                State::Committed as i32,
                "a start was sent: SIM-2 may fly"
            );
        } else {
            assert_eq!(state, State::Unassigned as i32, "{step}");
        }
    }
}

#[test]
fn a_receipt_of_the_aborted_run_takes_no_step_of_the_next_plan() {
    let (mut fleet, _) = running_fleet(true);
    let next = released_to_idle_sim2(&mut fleet);
    let receipt = deliver(&mut fleet, &next);
    abort(&mut fleet);
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "again".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("again"),
    );
    assert!(!reply.intents.is_empty(), "a new proposal goes out");
    let late = fleet
        .team
        .on_member_receipt("SIM-2", &receipt, &context("late"));
    assert!(
        late.intents.is_empty() && late.receipt.is_none(),
        "{late:?}"
    );
}

#[test]
fn an_abort_keeps_moved_lines_that_were_flown() {
    let (mut fleet, _) = running_fleet(true);
    let first = released_to_idle_sim2(&mut fleet);
    fleet.route(vec![first]);
    // SIM-2 flies the moved lines to the end; SIM-1 still flies its own.
    complete(&mut fleet, "SIM-2");
    abort(&mut fleet);
    assert_ne!(
        handover(&fleet).state,
        State::Unassigned as i32,
        "{}",
        handover(&fleet).basis
    );
}

#[test]
fn a_start_refused_after_an_abort_leaves_the_work_unassigned() {
    let (mut fleet, _) = running_fleet(true);
    let propose = released_to_idle_sim2(&mut fleet);
    let receipt = deliver(&mut fleet, &propose);
    let approve = fleet
        .team
        .on_member_receipt("SIM-2", &receipt, &context("r"))
        .intents
        .remove(0);
    let receipt = deliver(&mut fleet, &approve);
    let start = fleet
        .team
        .on_member_receipt("SIM-2", &receipt, &context("r"))
        .intents
        .remove(0);
    abort(&mut fleet);
    assert_eq!(
        handover(&fleet).state,
        State::Committed as i32,
        "SIM-2 may fly"
    );
    let refused = wire::MissionReceipt {
        request_id: start.intent.request_id.clone(),
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        reason: "aborted".to_owned(),
        lane: "SIM-2".to_owned(),
        ..wire::MissionReceipt::default()
    };
    let reply = fleet
        .team
        .on_member_receipt("SIM-2", &refused, &context("late"));
    assert!(reply.intents.is_empty());
    assert_eq!(
        handover(&fleet).state,
        State::Unassigned as i32,
        "it never started"
    );
}

#[test]
fn a_late_release_unassigns_only_the_lines_it_released() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    let moving = handover(&fleet).task_ids;
    // SIM-1 begins line 2, which moves, before its revision arrives.
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    lane.on_progress(&toward(lane, 3));
    fleet.route(intents);
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    assert!(matches!(
        lane.on_progress(&toward(lane, 4)),
        Some(LaneEffect::Start(_))
    ));
    let status = lane.take_changed().expect("changed");
    abort(&mut fleet);
    fleet.team.on_member_status(&status, &context("late"));
    let recorded = handover(&fleet);
    assert_eq!(recorded.state, State::Unassigned as i32);
    assert_eq!(
        recorded.task_ids.len(),
        moving.len() - 1,
        "the line SIM-1 began stays its own"
    );
}
