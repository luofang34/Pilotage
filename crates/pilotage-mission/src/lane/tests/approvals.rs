//! The run in progress and a waiting successor each hold their own
//! approval: a renewal of one never extends or revokes the other, and the
//! switch checks the successor's own.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

const MINUTE: u64 = 60_000;

fn at(now_unix_ms: u64, id: &str) -> IntentContext {
    IntentContext {
        now_unix_ms,
        fresh_id: id.to_owned(),
    }
}

/// An approval of `digest` for `run_id` (empty for a plan or a successor)
/// that expires at `expires`.
fn approval_until(digest: &[u8], run_id: &str, expires: u64) -> wire::MissionApproval {
    let mut approved = approval(digest, 120.0);
    approved.run_id = run_id.to_owned();
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.expires_unix_ms = expires;
    }
    approved
}

/// A lane flying line 1 of a plan whose approval expires at `expires`, and
/// a successor flown at 100 m, approved until `successor_expires` and
/// waiting for the end of line 1.
fn with_successor(expires: u64, successor_expires: u64) -> Lane {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    lane.on_intent(
        &intent(Action::Approve(approval_until(&digest, "", expires))),
        &context("a"),
    );
    lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: digest,
        })),
        &context("run-1"),
    );
    lane.on_progress(&toward(&lane, 1));
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
    let admitted = lane.on_intent(
        &intent(Action::Approve(approval_until(
            &draft,
            "",
            successor_expires,
        ))),
        &context("a2"),
    );
    assert!(accepted(&admitted), "{}", admitted.receipt.reason);
    assert!(admitted.effect.is_none(), "it waits for the end of line 1");
    lane
}

fn renew(lane: &mut Lane, run_id: &str, until: u64, now: u64) -> crate::lane::LaneReply {
    let digest = lane.status().proposal_digest;
    lane.on_intent(
        &intent(Action::Approve(approval_until(&digest, run_id, until))),
        &at(now, "renew"),
    )
}

#[test]
fn a_renewal_of_the_run_does_not_carry_a_successor_whose_approval_expired() {
    let mut lane = with_successor(NOW + 60 * MINUTE, NOW + 10 * MINUTE);
    lane.on_clock(NOW + 20 * MINUTE);
    let run_id = lane.status().run_id;
    let renewed = renew(&mut lane, &run_id, NOW + 120 * MINUTE, NOW + 20 * MINUTE);
    assert!(accepted(&renewed), "{}", renewed.receipt.reason);
    // Line 1 ends: the successor's own approval has expired.
    let effect = lane.on_progress(&toward(&lane, 3));
    assert!(effect.is_none(), "{effect:?}");
    assert_eq!(
        lane.status().successor.map(|s| s.state),
        Some(wire::MissionSuccessorState::Drafted as i32),
        "it waits for an approval of its own again"
    );
}

#[test]
fn a_valid_successor_approval_does_not_hide_the_runs_own_expiry() {
    let mut lane = with_successor(NOW + 10 * MINUTE, NOW + 120 * MINUTE);
    assert_eq!(
        lane.on_clock(NOW + 10 * MINUTE),
        Some(LaneEffect::Hold),
        "the run follows the expiry of its own approval"
    );
}

#[test]
fn after_the_switch_the_run_holds_the_successors_approval() {
    let mut lane = with_successor(NOW + 60 * MINUTE, NOW + 120 * MINUTE);
    let run_id = lane.status().run_id;
    // A renewal for another run changes nothing.
    assert!(!accepted(&renew(
        &mut lane,
        "another-run",
        NOW + 30 * MINUTE,
        NOW + MINUTE
    )));
    // The run's renewal, until +30 min, is the run's alone.
    assert!(accepted(&renew(
        &mut lane,
        &run_id,
        NOW + 30 * MINUTE,
        NOW + MINUTE
    )));
    assert!(matches!(
        lane.on_progress(&toward(&lane, 3)),
        Some(LaneEffect::Start(_))
    ));
    assert_eq!(
        lane.on_clock(NOW + 31 * MINUTE),
        None,
        "the successor's run is under the successor's approval"
    );
    assert_eq!(lane.on_clock(NOW + 120 * MINUTE), Some(LaneEffect::Hold));
}

#[test]
fn the_status_names_the_recoveries_of_the_approval_in_force() {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    let none = lane.status();
    assert_eq!(none.on_control_lost, 0, "no approval is in force");
    assert_eq!(none.on_expiry, 0);
    let mut approved = approval(&digest, 120.0);
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.on_control_lost = wire::MissionControlLossPolicy::ReturnHome as i32;
        envelope.on_expiry = wire::MissionExpiryPolicy::Hold as i32;
    }
    assert!(accepted(
        &lane.on_intent(&intent(Action::Approve(approved)), &context("a"))
    ));
    let status = lane.status();
    assert_eq!(
        status.on_control_lost(),
        wire::MissionControlLossPolicy::ReturnHome
    );
    assert_eq!(status.on_expiry(), wire::MissionExpiryPolicy::Hold);
}
