//! Every entry point of a team admits an approval with the checks a vehicle
//! lane makes, and a refusal changes nothing and sends nothing.

use pilotage_mission::{Approvable, admit_approval};
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::revision::{reassign, running_fleet};
use super::*;
use crate::TeamReply;

/// Builds a state and the approval its entry point admits.
type Entry = fn() -> (Fleet, wire::MissionApproval);

/// The ways an approval fails the checks every entry point makes.
fn spoiled(good: &wire::MissionApproval) -> Vec<(&'static str, wire::MissionApproval)> {
    let with = |change: fn(&mut wire::MissionApproval)| {
        let mut approval = good.clone();
        change(&mut approval);
        approval
    };
    vec![
        ("no format", with(|a| a.proposal_format = 0)),
        ("an unknown format", with(|a| a.proposal_format = 99)),
        ("another digest", with(|a| a.proposal_digest = vec![7; 32])),
        ("another run", with(|a| a.run_id = "run-9".to_owned())),
        (
            "an expired approval",
            with(|a| {
                if let Some(envelope) = a.envelope.as_mut() {
                    envelope.expires_unix_ms = 1;
                }
            }),
        ),
    ]
}

fn approve(fleet: &mut Fleet, approval: wire::MissionApproval) -> TeamReply {
    fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-approve".to_owned(),
            action: Some(Action::Approve(approval)),
        },
        &context("approve"),
    )
}

fn proposed() -> (Fleet, wire::MissionApproval) {
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
    (fleet, approval(&proposal.digest, 2))
}

fn renewal() -> (Fleet, wire::MissionApproval) {
    let (fleet, _) = running_fleet(true);
    let status = fleet.team.status();
    let mut renewed = approval(&status.proposal_digest, 2);
    renewed.run_id = status.run_id;
    (fleet, renewed)
}

fn successor() -> (Fleet, wire::MissionApproval) {
    let (mut fleet, _) = running_fleet(true);
    let draft = reassign(&fleet, "SIM-1", "SIM-2", 0.5, false);
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "operator-revise".to_owned(),
            action: Some(draft),
        },
        &context("revision"),
    );
    let draft = reply
        .receipt
        .as_ref()
        .and_then(opened)
        .expect("the draft for review");
    (fleet, approval(&draft.digest, 0))
}

/// What the team and its members show.
fn shown(fleet: &Fleet) -> (wire::MissionStatus, Vec<wire::MissionStatus>) {
    let members = fleet.members.values().map(Lane::status).collect();
    (fleet.team.status(), members)
}

#[test]
fn every_team_entry_refuses_a_spoiled_approval_and_changes_nothing() {
    let entries: [(&str, Entry); 3] = [
        ("plan", proposed),
        ("renewal", renewal),
        ("successor", successor),
    ];
    for (entry, build) in entries {
        let (_, good) = build();
        for (what, spoiled) in spoiled(&good) {
            let (mut fleet, _) = build();
            let before = shown(&fleet);
            let reply = approve(&mut fleet, spoiled);
            let receipt = reply.receipt.expect("refused at once");
            assert!(!accepted(&receipt), "{entry} took {what}");
            assert!(reply.intents.is_empty(), "{entry} passed on {what}");
            assert_eq!(shown(&fleet), before, "{entry} changed on {what}");
        }
        // The same entry admits the approval the spoiled ones came from.
        let (mut fleet, good) = build();
        let reply = approve(&mut fleet, good);
        assert!(
            !reply.intents.is_empty() || reply.receipt.as_ref().is_some_and(accepted),
            "{entry}: {:?}",
            reply.receipt
        );
    }
}

#[test]
fn the_team_refuses_with_the_verdict_a_lane_gives() {
    let (mut fleet, good) = proposed();
    let held = Approvable {
        plan: &good.proposal_digest,
        run: None,
        drafted: None,
    };
    for (what, spoiled) in spoiled(&good) {
        let verdict = admit_approval(&spoiled, held, NOW).expect_err(what);
        let receipt = approve(&mut fleet, spoiled).receipt.expect("refused");
        assert_eq!(receipt.reason, verdict, "{what}");
    }
}

#[test]
fn a_share_answered_after_the_team_approval_expired_gets_no_approval_passed_down() {
    let (mut fleet, good) = successor();
    let reply = approve(&mut fleet, good);
    let share = reply
        .intents
        .into_iter()
        .find(|i| i.member == "SIM-1")
        .expect("SIM-1 drafts its share");
    let lane = fleet.members.get_mut("SIM-1").expect("SIM-1");
    let drafted = lane.on_intent(&share.intent, &context("share")).receipt;
    let expired = IntentContext {
        now_unix_ms: NOW + 3_600_000,
        fresh_id: "late".to_owned(),
    };
    let reply = fleet.team.on_member_receipt("SIM-1", &drafted, &expired);
    assert!(reply.intents.is_empty(), "nothing is passed down");
    let status = fleet.team.status();
    assert!(status.detail.contains("not approved"), "{}", status.detail);
    assert!(
        status
            .handovers
            .iter()
            .all(|h| h.state != wire::MissionHandoverState::Committed as i32),
        "the handover is not committed"
    );
}
