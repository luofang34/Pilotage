#![allow(clippy::expect_used, clippy::panic)]

//! Per-scope link-loss behavior at the PX4 adapter boundary: a gimbal-scope
//! failsafe queues a best-effort zero-rate stop WITHOUT touching flight, a
//! motion-scope failsafe neutralizes flight without freezing the gimbal, and a
//! refused enactment is a typed fault that still latches its scope.

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_protocol::{LogicalAxisId, ScopeId, VehicleId};

use super::Px4Adapter;
use super::gimbal_tests::{gimbal_control, gimbal_frame, next_command};
use super::tests::{fake_fc, frame, live_state, uplink_to};

/// A full-coverage neutral flight frame (every declared axis reported at
/// zero), so a live adapter accepts it as ordinary motion control.
fn neutral_flight_frame() -> pilotage_protocol::ScopedControlFrame {
    frame(
        vec![
            (LogicalAxisId::new(super::ROLL_AXIS), 0.0),
            (LogicalAxisId::new(super::PITCH_AXIS), 0.0),
            (LogicalAxisId::new(super::THROTTLE_AXIS), 0.0),
            (LogicalAxisId::new(super::YAW_AXIS), 0.0),
        ],
        vec![],
    )
}

#[test]
fn gimbal_link_loss_latches_gimbal_but_leaves_motion_flying() {
    // The scope-specific link-loss contract (ADR-0008): losing the gimbal
    // scope fails closed on gimbal frames WITHOUT ever neutralizing the FC
    // or suppressing motion. Engaging the gimbal scope must not engage the
    // vehicle-wide flight policy.
    let (fc, addr) = fake_fc();
    fc.set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .expect("timeout");
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state())
        .with_uplink(uplink_to(addr))
        .with_gimbal(control);

    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(super::GIMBAL_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("the reachable gimbal accepts the zero-rate failsafe");

    // The gimbal scope's own latch suppresses gimbal frames...
    let gimbal = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        gimbal.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the gimbal latch suppresses gimbal frames"
    );

    // ...but motion is untouched: a neutral flight frame still flies.
    let motion = adapter.apply_control(&neutral_flight_frame());
    assert_eq!(
        motion.disposition,
        Disposition::Accepted,
        "a gimbal failsafe must never suppress motion"
    );

    // And engaging the gimbal scope never sent a neutral to the FC.
    let mut buf = [0u8; 128];
    assert!(
        fc.recv_from(&mut buf).is_err(),
        "engaging the gimbal scope must not neutralize the FC"
    );
}

#[test]
fn gimbal_link_loss_engage_queues_a_zero_rate_and_reasserts_the_claim() {
    // Engaging must actively STOP the slew, not merely latch: a nonzero rate
    // in flight would otherwise coast until the far slower stale-demand cutoff.
    // The failsafe re-asserts the primary-control claim (PX4 drops a rate from a
    // non-primary sender) and QUEUES a zero-rate to the FC's lanes — best-effort
    // (not FC-confirmed), so the test observes the queued lane traffic.
    let (control, mut lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);

    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(super::GIMBAL_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("the gimbal accepts the zero-rate failsafe");

    assert_eq!(
        next_command(&mut lanes),
        1001,
        "the failsafe re-asserts the primary-control claim"
    );
    let rate = lanes
        .rates
        .borrow_and_update()
        .expect("a zero-rate demand was published");
    assert_eq!(
        (rate.pitch_rps, rate.yaw_rps),
        (0.0, 0.0),
        "the failsafe queues a zero-rate setpoint"
    );
}

#[test]
fn a_refused_gimbal_zero_rate_is_a_typed_failure_but_still_latches() {
    // Fail-closed: if the zero-rate cannot reach its lane the enactment is a
    // typed failure, yet the latch still engages so gimbal frames stay
    // suppressed (a counted fault at the host, never a silent success).
    let (control, lanes) = gimbal_control();
    drop(lanes); // both the command and rate receivers are gone
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);

    let result = adapter.set_link_loss_policy(
        VehicleId::new(1),
        &ScopeId::new(super::GIMBAL_SCOPE),
        Some(LinkLossPolicy::Neutralize),
    );
    assert!(
        matches!(
            result,
            Err(pilotage_adapter_api::LinkLossEnactError::ChannelRejected { .. })
        ),
        "a refused zero-rate send is a typed failure, got {result:?}"
    );
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the latch engages even when the zero-rate send was refused"
    );
}

#[test]
fn a_dropped_gimbal_stop_is_a_counted_failure_that_still_latches() {
    // Fault injection (Simulation-only): dropping the host's zero-rate stop
    // must NOT masquerade as a successful enactment. The engage returns a
    // typed `ChannelRejected` — the fault the host counts — while the latch
    // still suppresses gimbal frames, and nothing leaves the host so PX4's own
    // setpoint-timeout is the sole failsafe under test.
    let (control, mut lanes) = gimbal_control();
    let control = control.with_dropped_link_loss_stop(true);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);

    let result = adapter.set_link_loss_policy(
        VehicleId::new(1),
        &ScopeId::new(super::GIMBAL_SCOPE),
        Some(LinkLossPolicy::Neutralize),
    );
    assert!(
        matches!(
            result,
            Err(pilotage_adapter_api::LinkLossEnactError::ChannelRejected { .. })
        ),
        "a dropped stop is a typed failure the host counts, got {result:?}"
    );
    assert!(
        lanes.commands.try_recv().is_err(),
        "a dropped stop sends no claim command"
    );
    assert!(
        !lanes.rates.has_changed().expect("rate lane open"),
        "a dropped stop publishes no zero-rate"
    );
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the latch stays engaged even though the stop was dropped"
    );
}

#[test]
fn motion_link_loss_latches_motion_but_leaves_the_gimbal_pointing() {
    // The converse: losing the motion scope neutralizes flight and latches
    // motion frames, while the gimbal keeps pointing — one scope's failsafe
    // never reaches the other.
    let (_fc, addr) = fake_fc();
    let (control, mut lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state())
        .with_uplink(uplink_to(addr))
        .with_gimbal(control);

    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(super::FLIGHT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("motion engage neutralizes the reachable FC");

    // The motion scope's latch suppresses motion frames...
    let motion = adapter.apply_control(&neutral_flight_frame());
    assert_eq!(
        motion.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the motion latch suppresses motion frames"
    );

    // ...but the gimbal keeps pointing.
    let gimbal = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        gimbal.disposition,
        Disposition::Accepted,
        "a motion failsafe must never suppress the gimbal"
    );
    assert!(
        lanes.commands.try_recv().is_ok(),
        "the gimbal still emits its claim/demand while motion is latched"
    );
}
