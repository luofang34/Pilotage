//! An approval that expires while its run flies: the recovery it names
//! applies once, a held run resumes only under a new approval, and a
//! waiting revision does not apply under the expired one.

use pilotage_protocol::wire;
use wire::MissionHoldReason as Reason;
use wire::mission_intent::Action;

use super::*;

/// The time the test approvals expire.
const EXPIRY: u64 = NOW + 3_600_000;

fn at(now_unix_ms: u64, id: &str) -> IntentContext {
    IntentContext {
        now_unix_ms,
        fresh_id: id.to_owned(),
    }
}

/// A renewal of `digest` for the run `run_id`, valid for two more hours
/// from `EXPIRY`.
fn renewal(digest: &[u8], run_id: &str) -> wire::MissionApproval {
    let mut renewed = approval(digest, 120.0);
    renewed.run_id = run_id.to_owned();
    if let Some(envelope) = renewed.envelope.as_mut() {
        envelope.expires_unix_ms = EXPIRY + 7_200_000;
    }
    renewed
}

/// A running lane whose approval names `policy`.
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

fn progress(lane: &Lane, target: usize) -> RunProgress {
    toward(lane, target)
}

#[test]
fn a_run_holds_when_its_approval_expires_and_resumes_only_when_renewed() {
    let (mut lane, _) = running_lane();
    assert_eq!(lane.on_clock(EXPIRY - 1), None);
    assert_eq!(lane.on_clock(EXPIRY), Some(LaneEffect::Hold));
    assert_eq!(lane.state(), wire::MissionLaneState::HoldRequested);
    assert_eq!(lane.on_clock(EXPIRY + 1), None, "the recovery applies once");
    let resume = || intent(Action::Resume(wire::MissionResume {}));
    let refused = lane.on_intent(&resume(), &at(EXPIRY + 2, "x"));
    assert!(!accepted(&refused));
    assert!(
        refused.receipt.reason.contains("approve the plan again"),
        "{}",
        refused.receipt.reason
    );
    let status = lane.status();
    assert_eq!(status.holds, vec![Reason::ApprovalExpired as i32]);
    let renewed = lane.on_intent(
        &intent(Action::Approve(renewal(
            &status.proposal_digest,
            &status.run_id,
        ))),
        &at(EXPIRY + 3, "a2"),
    );
    assert!(accepted(&renewed), "{}", renewed.receipt.reason);
    assert_eq!(
        renewed.effect,
        Some(LaneEffect::Resume),
        "held only for the expiry, the run resumes"
    );
    assert_eq!(lane.state(), wire::MissionLaneState::Running);
}

#[test]
fn a_renewal_ends_only_the_hold_of_the_expiry() {
    let (mut lane, _) = running_lane();
    lane.on_intent(&intent(Action::Hold(wire::MissionHold {})), &context("h"));
    assert_eq!(lane.on_clock(EXPIRY), None, "already asked to hold");
    let status = lane.status();
    assert_eq!(
        status.holds,
        vec![Reason::Operator as i32, Reason::ApprovalExpired as i32]
    );
    let resume = || intent(Action::Resume(wire::MissionResume {}));
    let refused = lane.on_intent(&resume(), &at(EXPIRY + 1, "x"));
    assert!(!accepted(&refused), "the expiry still holds it");
    let renewed = lane.on_intent(
        &intent(Action::Approve(renewal(
            &status.proposal_digest,
            &status.run_id,
        ))),
        &at(EXPIRY + 2, "a2"),
    );
    assert!(accepted(&renewed), "{}", renewed.receipt.reason);
    assert_eq!(renewed.effect, None, "the operator's hold stays");
    assert_eq!(lane.status().holds, vec![Reason::Operator as i32]);
    let resumed = lane.on_intent(&resume(), &at(EXPIRY + 3, "y"));
    assert_eq!(resumed.effect, Some(LaneEffect::Resume));
}

#[test]
fn a_renewal_names_the_run_it_renews() {
    let (mut lane, _) = running_lane();
    assert_eq!(lane.on_clock(EXPIRY), Some(LaneEffect::Hold));
    let digest = lane.status().proposal_digest;
    for other in ["", "an-earlier-run"] {
        let refused = lane.on_intent(
            &intent(Action::Approve(renewal(&digest, other))),
            &at(EXPIRY + 1, "a2"),
        );
        assert!(!accepted(&refused), "run {other:?}");
        assert!(
            refused.receipt.reason.contains("a renewal names the run"),
            "{}",
            refused.receipt.reason
        );
    }
    assert_eq!(lane.state(), wire::MissionLaneState::HoldRequested);
}

#[test]
fn a_held_run_whose_approval_expired_does_not_resume() {
    let (mut lane, _) = running_lane();
    assert!(accepted(&lane.on_intent(
        &intent(Action::Hold(wire::MissionHold {})),
        &context("h")
    )));
    let refused = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &at(EXPIRY, "x"),
    );
    assert!(!accepted(&refused));
    assert_eq!(lane.state(), wire::MissionLaneState::HoldRequested);
}

#[test]
fn a_return_home_policy_flies_home_instead_of_stopping() {
    let mut lane = running_under(wire::MissionExpiryPolicy::ReturnHome);
    lane.on_progress(&progress(&lane, 0));
    let Some(LaneEffect::Start(order)) = lane.on_clock(EXPIRY) else {
        panic!("between lines the way home starts at once");
    };
    assert!(order.run_id.ends_with("-rev1"), "{}", order.run_id);
    assert_eq!(lane.on_clock(EXPIRY + 1), None, "the recovery applies once");
    assert_eq!(lane.on_clock(EXPIRY + 2), None);
    assert_eq!(lane.state(), wire::MissionLaneState::Running);
    assert!(
        lane.status().detail.contains("the approval expired"),
        "{}",
        lane.status().detail
    );
}

#[test]
fn a_waiting_revision_does_not_apply_under_an_expired_approval() {
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    lane.on_progress(&progress(&lane, 1));
    let status = lane.status();
    let revise = intent(Action::Revise(wire::MissionRevise {
        base_digest: status.proposal_digest.clone(),
        run_id: status.run_id.clone(),
        within_envelope: true,
        drafted_by: None,
        release: None,
        change: Some(wire::mission_revise::Change::Finish(wire::MissionFinish {})),
    }));
    let drafted = lane.on_intent(&revise, &context("rev"));
    assert!(accepted(&drafted), "{}", drafted.receipt.reason);
    assert_eq!(drafted.effect, None, "it waits for the line in progress");
    assert_eq!(lane.on_clock(EXPIRY), Some(LaneEffect::Hold));
    // The line ends, but the revision waits for a new approval.
    assert_eq!(lane.on_progress(&progress(&lane, 3)), None);
    assert_eq!(lane.state(), wire::MissionLaneState::HoldRequested);
}
