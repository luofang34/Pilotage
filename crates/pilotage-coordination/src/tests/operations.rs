//! An operator request the team answers once its members answered is an
//! operation: one of a kind at a time, one receipt each, whatever order,
//! repetition or loss its members' answers come in.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::running_fleet;
use super::*;
use crate::TeamReply;

/// The operator's renewal of the running team's approval, as `request_id`.
fn renew(fleet: &mut Fleet, request_id: &str) -> TeamReply {
    let status = fleet.team.status();
    let mut renewal = approval(&status.proposal_digest, 2);
    renewal.run_id.clone_from(&status.run_id);
    fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: request_id.to_owned(),
            action: Some(Action::Approve(renewal)),
        },
        &context(request_id),
    )
}

/// What `intent`'s member lane answers, not yet given to the team.
fn answer(fleet: &mut Fleet, intent: &MemberIntent) -> wire::MissionReceipt {
    let lane = fleet.members.get_mut(&intent.member).expect("member");
    lane.on_intent(&intent.intent, &context("member")).receipt
}

/// Gives the team `member`'s `receipt`; returns the operator receipt it
/// decided, if any.
fn tell(
    fleet: &mut Fleet,
    member: &str,
    receipt: &wire::MissionReceipt,
) -> Option<wire::MissionReceipt> {
    fleet
        .team
        .on_member_receipt(member, receipt, &context("answer"))
        .receipt
}

fn outcome(receipt: &wire::MissionReceipt) -> wire::MissionReceiptOutcome {
    wire::MissionReceiptOutcome::try_from(receipt.outcome).expect("an outcome")
}

#[test]
fn a_second_renewal_while_one_waits_is_busy_and_the_first_still_ends_once() {
    let (mut fleet, _) = running_fleet(false);
    let first = renew(&mut fleet, "renew-1");
    assert!(first.receipt.is_none(), "it waits for the members");
    assert_eq!(first.intents.len(), 3);
    let second = renew(&mut fleet, "renew-2");
    let busy = second.receipt.expect("answered at once");
    assert_eq!(outcome(&busy), wire::MissionReceiptOutcome::Refused);
    assert!(busy.reason.contains("busy"), "{}", busy.reason);
    assert!(second.intents.is_empty(), "no member hears of it");
    let decided = fleet.route(first.intents);
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(decided[0].request_id, "renew-1");
    assert_eq!(outcome(&decided[0]), wire::MissionReceiptOutcome::Accepted);
    // Once it ended, a renewal is taken again.
    let third = renew(&mut fleet, "renew-3");
    assert!(third.receipt.is_none() && !third.intents.is_empty());
}

#[test]
fn answers_in_any_order_and_repeated_end_the_operation_once() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    let answers: Vec<(String, wire::MissionReceipt)> = reply
        .intents
        .iter()
        .map(|i| (i.member.clone(), answer(&mut fleet, i)))
        .collect();
    let mut decided = Vec::new();
    // The last answer first, then each again.
    for (member, receipt) in answers.iter().rev().chain(answers.iter()) {
        decided.extend(tell(&mut fleet, member, receipt));
    }
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(outcome(&decided[0]), wire::MissionReceiptOutcome::Accepted);
    assert!(fleet.team.take_ended().is_empty());
}

#[test]
fn a_request_never_sent_is_refused_and_says_so() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    let (unsent, sent) = reply.intents.split_first().expect("intents");
    let mut decided: Vec<wire::MissionReceipt> = Vec::new();
    for intent in sent {
        let receipt = answer(&mut fleet, intent);
        decided.extend(tell(&mut fleet, &intent.member, &receipt));
    }
    decided.extend(
        fleet
            .team
            .on_member_unsent(
                &unsent.member,
                &unsent.intent.request_id,
                "the link is stopped",
                &context("unsent"),
            )
            .receipt,
    );
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(outcome(&decided[0]), wire::MissionReceiptOutcome::Refused);
    let expected = format!("not sent to {}", unsent.member);
    assert!(
        decided[0].reason.contains(&expected),
        "{}",
        decided[0].reason
    );
}

#[test]
fn an_answer_lost_with_the_link_makes_the_receipt_uncertain_and_a_late_one_answers_nobody() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    let (lost, rest) = reply.intents.split_first().expect("intents");
    for intent in rest {
        let receipt = answer(&mut fleet, intent);
        assert!(
            tell(&mut fleet, &intent.member, &receipt).is_none(),
            "one still owes its answer"
        );
    }
    let late = answer(&mut fleet, lost);
    fleet.team.on_member_lost(&lost.member);
    let ended = fleet.team.take_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].request_id, "renew");
    assert_eq!(outcome(&ended[0]), wire::MissionReceiptOutcome::Uncertain);
    assert!(
        ended[0].reason.contains("not known whether"),
        "{}",
        ended[0].reason
    );
    assert!(
        ended[0].reason.contains(&lost.member),
        "{}",
        ended[0].reason
    );
    // The member comes back and its answer arrives: it answers no operator.
    let status = fleet
        .members
        .get(&lost.member)
        .map(Lane::status)
        .expect("member");
    fleet.team.on_member_status(&status, &context("back"));
    assert!(tell(&mut fleet, &lost.member, &late).is_none());
    assert!(fleet.team.take_ended().is_empty());
}

#[test]
fn a_timeout_ends_the_wait_uncertain_and_a_late_answer_answers_nobody() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    fleet.team.on_host_clock(NOW + 29_000);
    assert!(fleet.team.take_ended().is_empty(), "not yet");
    fleet.team.on_host_clock(NOW + 30_000);
    let ended = fleet.team.take_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(outcome(&ended[0]), wire::MissionReceiptOutcome::Uncertain);
    assert!(
        ended[0].reason.contains("no answer within 30 s"),
        "{}",
        ended[0].reason
    );
    let decided = fleet.route(reply.intents);
    assert!(
        decided.is_empty(),
        "late answers answer nobody: {decided:?}"
    );
    fleet.team.on_host_clock(NOW + 90_000);
    assert!(fleet.team.take_ended().is_empty(), "ended once");
}

#[test]
fn an_abort_ends_an_open_operation_with_one_receipt() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    let abort = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "abort".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("abort"),
    );
    assert!(abort.receipt.is_some_and(|r| accepted(&r)));
    let ended = fleet.team.take_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].request_id, "renew");
    assert_eq!(outcome(&ended[0]), wire::MissionReceiptOutcome::Uncertain);
    assert!(ended[0].reason.contains("aborted"), "{}", ended[0].reason);
    let decided = fleet.route(reply.intents);
    assert!(
        decided.is_empty(),
        "late answers answer nobody: {decided:?}"
    );
    assert!(fleet.team.take_ended().is_empty());
}

#[test]
fn an_answer_lost_first_ends_the_operation_once_the_others_answered() {
    let (mut fleet, _) = running_fleet(false);
    let reply = renew(&mut fleet, "renew");
    let (lost, rest) = reply.intents.split_first().expect("intents");
    fleet.team.on_member_lost(&lost.member);
    assert!(
        fleet.team.take_ended().is_empty(),
        "others still owe answers"
    );
    let mut decided = Vec::new();
    for intent in rest {
        let receipt = answer(&mut fleet, intent);
        decided.extend(tell(&mut fleet, &intent.member, &receipt));
    }
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(outcome(&decided[0]), wire::MissionReceiptOutcome::Uncertain);
    assert!(
        decided[0].reason.contains(&lost.member),
        "{}",
        decided[0].reason
    );
    assert!(fleet.team.take_ended().is_empty(), "one receipt only");
    let next = renew(&mut fleet, "renew-2");
    assert!(next.receipt.is_none(), "not busy: {:?}", next.receipt);
}

#[test]
fn an_approval_whose_answer_is_lost_does_not_keep_the_team_waiting() {
    let mut fleet = Fleet::new();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-1".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("team-proposal"),
    );
    let proposal = fleet.route(reply.intents)[0]
        .proposal_record
        .clone()
        .and_then(open_record)
        .expect("proposal");
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-approve".to_owned(),
            action: Some(Action::Approve(approval(&proposal.digest, 2))),
        },
        &context("approve"),
    );
    assert!(reply.receipt.is_none() && !reply.intents.is_empty());
    fleet.team.on_host_clock(NOW + 30_000);
    let ended = fleet.team.take_ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(outcome(&ended[0]), wire::MissionReceiptOutcome::Uncertain);
    // The members never answer; the operator aborts and proposes again.
    let abort = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "abort".to_owned(),
            action: Some(Action::Abort(wire::MissionAbort {})),
        },
        &context("abort"),
    );
    assert!(abort.receipt.is_some_and(|r| accepted(&r)));
    let again = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-2".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("again"),
    );
    assert!(
        again.receipt.is_none() && !again.intents.is_empty(),
        "{:?}",
        again.receipt
    );
}

#[test]
fn a_late_answer_to_an_ended_renewal_does_not_take_the_next_ones_tally() {
    let (mut fleet, _) = running_fleet(false);
    let first = renew(&mut fleet, "renew-1");
    fleet.team.on_host_clock(NOW + 30_000);
    assert_eq!(fleet.team.take_ended().len(), 1, "renew-1 ended uncertain");
    let late: Vec<(String, wire::MissionReceipt)> = first
        .intents
        .iter()
        .map(|i| (i.member.clone(), answer(&mut fleet, i)))
        .collect();
    let second = renew(&mut fleet, "renew-2");
    let (refusing, rest) = second.intents.split_first().expect("intents");
    let refused = wire::MissionReceipt {
        request_id: refusing.intent.request_id.clone(),
        outcome: wire::MissionReceiptOutcome::Refused as i32,
        reason: "busy here".to_owned(),
        lane: refusing.member.clone(),
        ..wire::MissionReceipt::default()
    };
    let mut decided: Vec<wire::MissionReceipt> = Vec::new();
    decided.extend(tell(&mut fleet, &refusing.member, &refused));
    for (member, receipt) in &late {
        decided.extend(tell(&mut fleet, member, receipt));
    }
    for intent in rest {
        let receipt = answer(&mut fleet, intent);
        decided.extend(tell(&mut fleet, &intent.member, &receipt));
    }
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(decided[0].request_id, "renew-2");
    assert_eq!(outcome(&decided[0]), wire::MissionReceiptOutcome::Refused);
    assert!(
        decided[0].reason.contains(&refusing.member),
        "{}",
        decided[0].reason
    );
}
