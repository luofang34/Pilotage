//! A run that lost control: a principal took the lease, and that
//! principal's lease ended without a hand-back. Under a valid approval that
//! names the way home, the lane flies home once, as a recovery that never
//! runs the work again; otherwise it waits for a hand-back.

use pilotage_protocol::wire;
use wire::MissionHoldReason as Reason;
use wire::MissionLaneState as State;
use wire::mission_intent::Action;

use super::*;
use crate::lane::{RunExecution, RunOrder};

const EXPIRY: u64 = NOW + 3_600_000;

/// A running lane whose approval names `on_lost` and `on_expiry`.
fn running_under(
    on_lost: wire::MissionControlLossPolicy,
    on_expiry: wire::MissionExpiryPolicy,
) -> (Lane, Box<RunOrder>) {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(objective())), &context("p"));
    let digest = opened(&reply.receipt).expect("a proposal").digest;
    let mut approved = approval(&digest, 120.0);
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.on_control_lost = on_lost as i32;
        envelope.on_expiry = on_expiry as i32;
    }
    assert!(accepted(
        &lane.on_intent(&intent(Action::Approve(approved)), &context("a"))
    ));
    let start = Action::Start(wire::MissionStart {
        proposal_digest: digest,
    });
    let started = lane.on_intent(&intent(start), &context("r"));
    let Some(LaneEffect::Start(order)) = started.effect else {
        panic!("the run starts: {}", started.receipt.reason);
    };
    (lane, order)
}

/// The run's progress on line 1, with the control `holder` took and
/// whether that holder's lease ended without a hand-back.
fn lost(lane: &Lane, holder: u64, freed: bool) -> RunProgress {
    RunProgress {
        execution: Some(RunExecution::ControlLost {
            holder: Some(holder),
            freed,
        }),
        ..toward(lane, 1)
    }
}

#[test]
fn a_lost_control_left_free_flies_home_once_under_the_approved_recovery() {
    let (mut lane, work) = running_under(
        wire::MissionControlLossPolicy::ReturnHome,
        wire::MissionExpiryPolicy::Hold,
    );
    assert!(!work.recovery, "the work's run is not a recovery");
    // A person holds control: nothing moves against them.
    assert_eq!(lane.on_progress(&lost(&lane, 21, false)), None);
    assert_eq!(lane.state(), State::Running);
    // The person's lease ended without a hand-back.
    let Some(LaneEffect::Start(home)) = lane.on_progress(&lost(&lane, 21, true)) else {
        panic!("the approved recovery flies: {}", lane.status().detail);
    };
    assert!(home.recovery, "the run only returns home");
    assert_ne!(home.run_id, work.run_id);
    assert!(
        lane.status().detail.contains("returns to the launch point")
            || lane.status().detail.contains("return to the launch point"),
        "{}",
        lane.status().detail
    );
    // It applies once, whatever the replaced run still reports.
    assert_eq!(lane.on_progress(&lost(&lane, 21, true)), None);
    // The recovery takes the lease; a person takes it from the recovery and
    // leaves without a hand-back: the recovery starts again.
    let flying = RunProgress {
        execution: Some(RunExecution::Flying),
        ..RunProgress::default()
    };
    assert_eq!(lane.on_progress(&flying), None);
    let taken = RunProgress {
        execution: Some(RunExecution::ControlLost {
            holder: Some(22),
            freed: false,
        }),
        ..RunProgress::default()
    };
    assert_eq!(lane.on_progress(&taken), None, "the person wins");
    let left = RunProgress {
        execution: Some(RunExecution::ControlLost {
            holder: Some(22),
            freed: true,
        }),
        ..RunProgress::default()
    };
    let Some(LaneEffect::Start(again)) = lane.on_progress(&left) else {
        panic!("the recovery starts again: {}", lane.status().detail);
    };
    assert!(again.recovery);
    assert_ne!(again.run_id, home.run_id);
}

#[test]
fn without_an_approved_recovery_a_lost_control_waits_for_a_hand_back() {
    for policy in [
        wire::MissionControlLossPolicy::Unspecified,
        wire::MissionControlLossPolicy::Hold,
    ] {
        let (mut lane, _) = running_under(policy, wire::MissionExpiryPolicy::Hold);
        assert_eq!(lane.on_progress(&lost(&lane, 21, true)), None, "{policy:?}");
        assert_eq!(lane.state(), State::Running, "{policy:?}");
        assert!(lane.status().successor.is_none(), "{policy:?}");
    }
}

#[test]
fn after_the_approval_expired_only_its_expiry_recovery_applies() {
    let (mut lane, _) = running_under(
        wire::MissionControlLossPolicy::ReturnHome,
        wire::MissionExpiryPolicy::Hold,
    );
    assert_eq!(lane.on_clock(EXPIRY), Some(LaneEffect::Hold));
    assert_eq!(
        lane.on_progress(&lost(&lane, 21, true)),
        None,
        "the expired approval grants no recovery but its own"
    );
    assert!(lane.status().successor.is_none());
}

#[test]
fn an_operators_hold_still_holds_the_recovery() {
    let (mut lane, _) = running_under(
        wire::MissionControlLossPolicy::ReturnHome,
        wire::MissionExpiryPolicy::Hold,
    );
    assert!(accepted(&lane.on_intent(
        &intent(Action::Hold(wire::MissionHold {})),
        &context("h")
    )));
    let Some(LaneEffect::Start(home)) = lane.on_progress(&lost(&lane, 21, true)) else {
        panic!("the recovery replaces the run");
    };
    assert!(home.recovery);
    assert!(matches!(lane.state(), State::HoldRequested | State::Held));
    assert_eq!(lane.status().holds, [Reason::Operator as i32]);
}

#[test]
fn an_expiry_way_home_switches_at_once_while_control_is_lost() {
    let (mut lane, _) = running_under(
        wire::MissionControlLossPolicy::Hold,
        wire::MissionExpiryPolicy::ReturnHome,
    );
    // The run is on line 1, and a person holds control: no one flies the
    // line to its end, so the way home does not wait for it.
    assert_eq!(lane.on_progress(&lost(&lane, 21, false)), None);
    let Some(LaneEffect::Start(home)) = lane.on_clock(EXPIRY) else {
        panic!("the expiry's way home replaces the run");
    };
    assert!(
        home.recovery,
        "the host may start it once the lease is free"
    );
}
