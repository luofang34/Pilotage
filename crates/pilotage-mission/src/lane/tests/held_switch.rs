//! An operator's hold survives a switch of the plan: a revision that
//! reaches its checkpoint, or the return home an expired approval names,
//! replaces the run but leaves it held until a person resumes it.

use pilotage_protocol::wire;
use wire::MissionHoldReason as Reason;
use wire::MissionLaneState as State;
use wire::mission_intent::Action;

use super::*;

const EXPIRY: u64 = NOW + 3_600_000;

fn hold(lane: &mut Lane) {
    let held = lane.on_intent(&intent(Action::Hold(wire::MissionHold {})), &context("h"));
    assert!(accepted(&held), "{}", held.receipt.reason);
}

/// A running lane whose approval names `policy` on expiry.
fn running_under(policy: wire::MissionExpiryPolicy) -> Lane {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    let mut approved = approval(&digest, 120.0);
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.on_expiry = policy as i32;
    }
    assert!(accepted(
        &lane.on_intent(&intent(Action::Approve(approved)), &context("a"))
    ));
    let start = Action::Start(wire::MissionStart {
        proposal_digest: digest,
    });
    assert!(accepted(&lane.on_intent(&intent(start), &context("r"))));
    lane
}

fn holds(lane: &Lane) -> Vec<i32> {
    lane.status().holds
}

/// Drafts a revision of the run in progress that flies at 100 m.
fn draft_higher(lane: &mut Lane) -> Vec<u8> {
    let status = lane.status();
    let mut higher = objective();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = higher.kind.as_mut() {
        survey.height_m = 100.0;
    }
    let drafted = lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            change: Some(wire::mission_revise::Change::Objective(higher)),
            ..wire::MissionRevise::default()
        })),
        &context("rev"),
    );
    opened(&drafted.receipt).expect("a draft").digest
}

#[test]
fn a_revision_that_reaches_its_checkpoint_while_held_stays_held() {
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    lane.on_progress(&toward(&lane, 1));
    let draft = draft_higher(&mut lane);
    let admitted = lane.on_intent(
        &intent(Action::Approve(approval(&draft, 120.0))),
        &context("a2"),
    );
    assert!(accepted(&admitted), "{}", admitted.receipt.reason);
    assert_eq!(admitted.effect, None, "it waits for the line in progress");
    hold(&mut lane);
    // The line in progress ends: the revision replaces the run, held.
    let switched = lane.on_progress(&toward(&lane, 3));
    assert!(
        matches!(switched, Some(LaneEffect::Start(_))),
        "the revised run replaces the old one: {switched:?}"
    );
    assert!(
        matches!(lane.state(), State::HoldRequested | State::Held),
        "{:?}",
        lane.state()
    );
    assert_eq!(holds(&lane), [Reason::Operator as i32]);
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("r"),
    );
    assert!(accepted(&resumed), "{}", resumed.receipt.reason);
    assert_eq!(resumed.effect, Some(LaneEffect::Resume));
    assert_eq!(lane.state(), State::Running);
}

#[test]
fn an_approved_revision_admitted_while_held_stays_held() {
    let (mut lane, _) = running_lane();
    lane.on_progress(&toward(&lane, 1));
    hold(&mut lane);
    let draft = draft_higher(&mut lane);
    let admitted = lane.on_intent(
        &intent(Action::Approve(approval(&draft, 120.0))),
        &context("a2"),
    );
    assert!(accepted(&admitted), "{}", admitted.receipt.reason);
    assert!(matches!(admitted.effect, Some(LaneEffect::Start(_))));
    assert!(matches!(lane.state(), State::HoldRequested | State::Held));
    assert_eq!(
        holds(&lane),
        [Reason::Operator as i32],
        "the approval resumes nothing"
    );
}

#[test]
fn a_return_home_on_expiry_while_held_stays_held() {
    let mut lane = running_under(wire::MissionExpiryPolicy::ReturnHome);
    lane.on_progress(&toward(&lane, 0));
    hold(&mut lane);
    let recovered = lane.on_clock(EXPIRY);
    assert!(
        matches!(recovered, Some(LaneEffect::Start(_))),
        "the way home replaces the run: {recovered:?}"
    );
    assert!(matches!(lane.state(), State::HoldRequested | State::Held));
    assert_eq!(holds(&lane), [Reason::Operator as i32]);
    // The person's resume lets the way home fly, under the expired
    // approval that named it.
    let resumed = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &IntentContext {
            now_unix_ms: EXPIRY + 1,
            fresh_id: "r".to_owned(),
        },
    );
    assert!(accepted(&resumed), "{}", resumed.receipt.reason);
    assert_eq!(lane.state(), State::Running);
}
