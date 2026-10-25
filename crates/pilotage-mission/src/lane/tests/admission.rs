//! Every entry point of a vehicle lane admits an approval with the same
//! checks, and a refusal changes nothing.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;
use crate::lane::{Approvable, admit_approval};

/// Builds a state and the approval its entry point admits.
type Entry = fn() -> (Lane, wire::MissionApproval);

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

fn proposed() -> (Lane, wire::MissionApproval) {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    (lane, approval(&digest, 120.0))
}

fn renewal() -> (Lane, wire::MissionApproval) {
    let (lane, _) = running_lane();
    let mut renewed = approval(&lane.status().proposal_digest, 120.0);
    renewed.run_id = "run-1".to_owned();
    (lane, renewed)
}

fn successor() -> (Lane, wire::MissionApproval) {
    let (mut lane, _) = running_lane();
    let mut higher = objective();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = higher.kind.as_mut() {
        survey.height_m = 100.0;
    }
    let status = lane.status();
    let drafted = lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            change: Some(wire::mission_revise::Change::Objective(higher)),
            ..wire::MissionRevise::default()
        })),
        &context("rev"),
    );
    let draft = opened(&drafted.receipt).expect("a draft").digest;
    (lane, approval(&draft, 120.0))
}

#[test]
fn every_entry_refuses_a_spoiled_approval_and_changes_nothing() {
    let entries: [(&str, Entry); 3] = [
        ("plan", proposed),
        ("renewal", renewal),
        ("successor", successor),
    ];
    for (entry, build) in entries {
        let (_, good) = build();
        for (what, spoiled) in spoiled(&good) {
            let (mut lane, _) = build();
            let before = lane.status();
            let reply = lane.on_intent(&intent(Action::Approve(spoiled)), &context("bad"));
            assert!(!accepted(&reply), "{entry} took {what}");
            assert!(reply.effect.is_none(), "{entry}: {what}");
            assert_eq!(lane.status(), before, "{entry} changed on {what}");
        }
        // The same entry admits the approval the spoiled ones came from.
        let (mut lane, good) = build();
        let reply = lane.on_intent(&intent(Action::Approve(good)), &context("good"));
        assert!(accepted(&reply), "{entry}: {}", reply.receipt.reason);
    }
}

#[test]
fn the_lane_refuses_with_the_shared_verdict() {
    let (mut lane, good) = proposed();
    let held = Approvable {
        plan: &good.proposal_digest,
        run: None,
        drafted: None,
    };
    for (what, spoiled) in spoiled(&good) {
        let verdict = admit_approval(&spoiled, held, NOW).expect_err(what);
        let reply = lane.on_intent(&intent(Action::Approve(spoiled)), &context("bad"));
        assert_eq!(reply.receipt.reason, verdict, "{what}");
    }
}
