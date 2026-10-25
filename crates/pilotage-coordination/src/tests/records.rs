//! Handover records outlive the plan they came from: a new proposal, a
//! failed one or a member's refusal clears none, an unreconciled one stops
//! new work until a person resolves it, and only a settled one is dropped.

use pilotage_protocol::wire;
use wire::MissionHandoverState as State;
use wire::mission_intent::Action;

use super::handover::{complete, handover, move_sim1_to_sim2, sim1_switches};
use super::revision::running_fleet;
use super::*;

fn propose(fleet: &mut Fleet) -> (Option<wire::MissionReceipt>, Vec<MemberIntent>) {
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "again".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("again"),
    );
    (reply.receipt, reply.intents)
}

/// A fleet whose team committed a handover of SIM-1's lines to SIM-2, which
/// was busy, and then restarted: the journal it wrote is read back.
fn restarted_with_a_committed_handover() -> Fleet {
    let (mut fleet, _) = running_fleet(true);
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    fleet.team.on_member_status(&status, &context("released"));
    assert_eq!(handover(&fleet).state, State::Committed as i32);
    let journal = fleet.team.handovers();
    fleet.team = team();
    fleet.team.restore_handovers(&journal);
    fleet
}

#[test]
fn an_unreconciled_handover_stops_new_work_and_survives_it() {
    let mut fleet = restarted_with_a_committed_handover();
    let (receipt, intents) = propose(&mut fleet);
    let receipt = receipt.expect("answered at once");
    assert!(!accepted(&receipt));
    assert!(
        receipt.reason.contains("unreconciled"),
        "{}",
        receipt.reason
    );
    assert!(intents.is_empty(), "no member hears of new work");
    assert_eq!(handover(&fleet).state, State::Unreconciled as i32);
    // Written and read back again, after a second restart, it is still there.
    let journal = fleet.team.handovers();
    fleet.team = team();
    fleet.team.restore_handovers(&journal);
    assert_eq!(handover(&fleet).state, State::Unreconciled as i32);
}

#[test]
fn a_person_resolves_the_handover_and_then_new_work_goes_out() {
    let mut fleet = restarted_with_a_committed_handover();
    let assignment_id = handover(&fleet).assignment_id;
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "resolve".to_owned(),
            action: Some(Action::ResolveHandover(wire::MissionResolveHandover {
                assignment_id,
                resolved_by: None,
            })),
        },
        &context("resolve"),
    );
    assert!(reply.receipt.is_some_and(|r| accepted(&r)));
    assert_eq!(handover(&fleet).state, State::Resolved as i32);
    let (receipt, intents) = propose(&mut fleet);
    assert!(
        receipt.is_none() && !intents.is_empty(),
        "the proposal goes out"
    );
    assert!(
        fleet.team.status().handovers.is_empty(),
        "a resolved record is dropped with the old plan"
    );
}

#[test]
fn a_proposal_a_member_refuses_keeps_an_unassigned_record() {
    let (mut fleet, _) = running_fleet(true);
    complete(&mut fleet, "SIM-2");
    let intents = move_sim1_to_sim2(&mut fleet);
    fleet.route(intents);
    let status = sim1_switches(&mut fleet);
    let sent = fleet.team.on_member_status(&status, &context("released"));
    let to_sim2 = sent
        .into_iter()
        .find(|i| i.member == "SIM-2")
        .expect("SIM-2");
    let refused = wire::MissionReceipt {
        request_id: to_sim2.intent.request_id,
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        reason: "no".to_owned(),
        lane: "SIM-2".to_owned(),
        ..wire::MissionReceipt::default()
    };
    fleet
        .team
        .on_member_receipt("SIM-2", &refused, &context("no"));
    assert_eq!(handover(&fleet).state, State::Unassigned as i32);
    // The team ends its run, and a new proposal is refused by a member.
    fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "abort".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("abort"),
    );
    let (_, intents) = propose(&mut fleet);
    let first = intents.first().cloned().expect("a member proposal");
    let refused = wire::MissionReceipt {
        request_id: first.intent.request_id,
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        reason: "busy".to_owned(),
        lane: first.member.clone(),
        ..wire::MissionReceipt::default()
    };
    fleet
        .team
        .on_member_receipt(&first.member, &refused, &context("busy"));
    assert_eq!(
        handover(&fleet).state,
        State::Unassigned as i32,
        "it is still unresolved"
    );
}

#[test]
fn a_live_handover_of_an_ended_run_stops_the_next_plan_until_resolved() {
    for released in [true, false] {
        let (mut fleet, _) = running_fleet(true);
        let intents = move_sim1_to_sim2(&mut fleet);
        fleet.route(intents);
        if released {
            let status = sim1_switches(&mut fleet);
            fleet.team.on_member_status(&status, &context("released"));
            assert_eq!(handover(&fleet).state, State::Committed as i32);
        } else {
            assert_eq!(handover(&fleet).state, State::Releasing as i32);
        }
        fleet.team.on_intent(
            &wire::MissionIntent {
                request_id: "abort".to_owned(),
                action: Some(Action::Abort(wire::MissionAbort {})),
            },
            &context("abort"),
        );
        let (receipt, intents) = propose(&mut fleet);
        if released {
            // The abort cancelled the moved flight before it started: the
            // lines are known unflown, so they wait for a member and do not
            // stop the next plan.
            assert_eq!(handover(&fleet).state, State::Unassigned as i32);
            assert!(receipt.is_none_or(|r| accepted(&r)) && !intents.is_empty());
            continue;
        }
        let receipt = receipt.expect("refused at once");
        assert!(
            !accepted(&receipt) && intents.is_empty(),
            "{}",
            receipt.reason
        );
        assert_eq!(handover(&fleet).state, State::Unreconciled as i32);
    }
}

#[test]
fn reading_the_journal_back_twice_keeps_one_record_of_each_handover() {
    let mut fleet = restarted_with_a_committed_handover();
    let journal = fleet.team.handovers();
    fleet.team.restore_handovers(&journal);
    assert_eq!(fleet.team.status().handovers.len(), 1);
}

#[test]
fn work_moved_on_a_statement_and_then_not_flown_stops_the_next_plan() {
    use super::revision::{member_down, reassign, team_intent};
    let (mut fleet, _) = running_fleet(false);
    // SIM-3's link is gone and a person states it down: its lines move to
    // SIM-1 without a release, as SIM-3 cannot answer.
    fleet.team.on_member_lost("SIM-3");
    let (receipt, _) = team_intent(&mut fleet, member_down("SIM-3"));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    let give = reassign(&fleet, "SIM-3", "SIM-1", 1.0, false);
    let (receipt, _) = team_intent(&mut fleet, give);
    let draft = opened(&receipt.expect("answered")).expect("a draft");
    let (receipt, _) = team_intent(&mut fleet, Action::Approve(approval(&draft.digest, 0)));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    assert_eq!(handover(&fleet).state, State::Committed as i32);
    // The team aborts before SIM-1 starts the moved lines.
    let (receipt, _) = team_intent(&mut fleet, Action::Abort(wire::MissionAbort {}));
    assert!(receipt.is_some_and(|r| accepted(&r)));
    let record = handover(&fleet);
    assert_eq!(
        record.state,
        State::Unreconciled as i32,
        "SIM-3 released nothing and may still fly its lines: {}",
        record.basis
    );
    let (receipt, intents) = propose(&mut fleet);
    let receipt = receipt.expect("refused at once");
    assert!(
        !accepted(&receipt) && intents.is_empty(),
        "{}",
        receipt.reason
    );
}
