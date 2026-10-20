//! Per-scope link-loss behavior at the PX4 adapter boundary: a gimbal-scope
//! failsafe queues a best-effort zero-rate stop WITHOUT touching flight, a
//! motion-scope failsafe neutralizes flight without freezing the gimbal, and a
//! refused enactment is a typed fault that still latches its scope.

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_protocol::{LogicalAxisId, ScopeId, VehicleId};

use super::Px4Adapter;
use super::gimbal_tests::{gimbal_control, gimbal_frame};
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
    adapter.uplink.as_mut().expect("uplink").begin_arm(0.0);
    let mut initial = [0u8; 128];
    fc.recv_from(&mut initial).expect("initial setpoint");

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

    // Engaging the gimbal scope itself never sent a neutral to the FC.
    let mut buf = [0u8; 128];
    assert!(
        fc.recv_from(&mut buf).is_err(),
        "engaging the gimbal scope must not neutralize the FC"
    );

    // ...but motion is untouched: a neutral flight frame still flies.
    let motion = adapter.apply_control(&neutral_flight_frame());
    assert_eq!(
        motion.disposition,
        Disposition::Accepted,
        "a gimbal failsafe must never suppress motion"
    );
}

#[test]
fn gimbal_link_loss_fences_unsent_motion_without_inventing_a_physical_stop() {
    let (control, mut lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);

    let initial = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(initial.disposition, Disposition::Accepted);
    while lanes.commands.try_recv().is_ok() {}

    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(super::GIMBAL_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("the gimbal accepts ownership release");

    let release = lanes.commands.try_recv().expect("ownership release");
    assert_eq!(release.command, 1001);
    assert_eq!(&release.params[..2], &[-3.0, -3.0]);
    assert!(release.receipt.is_some());
    assert!(lanes.commands.try_recv().is_err());
    assert!(lanes.rates.borrow_and_update().is_none());
    let progress = adapter
        .gimbal
        .as_ref()
        .unwrap()
        .snapshot_cached()
        .unwrap()
        .progress;
    assert_eq!(progress.sent_at, None);
    assert_eq!(progress.ended, Some(turret_core::EndReason::Revoked));
}

#[test]
fn closed_transport_still_allows_unsent_motion_to_be_fenced() {
    let (control, lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);

    let initial = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(initial.disposition, Disposition::Accepted);
    drop(lanes);

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
        "pending motion is fenced but the ownership release cannot be queued"
    );
    let progress = adapter
        .gimbal
        .as_ref()
        .unwrap()
        .snapshot_cached()
        .unwrap()
        .progress;
    assert_eq!(progress.sent_at, None);
    assert_eq!(progress.ended, Some(turret_core::EndReason::Revoked));
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the latch also fences an unavailable transport"
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
        lanes.rates.borrow().is_none(),
        "a dropped stop leaves no queued motion"
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
