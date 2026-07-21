#![allow(clippy::expect_used, clippy::panic)]

//! Gimbal-scope (vehicle.gimbal) adapter behavior: capability
//! advertisement, pointing that works where flight is rejected, the
//! recenter command, axis validation, and the durable claim-denial
//! state that a later unrelated ack must not bury.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, ScopeId, VehicleId};

use pilotage_mavlink::LinkState;
use pilotage_mavlink::codec::FcMessage;
use pilotage_mavlink::link::apply_messages_at;

use super::Px4Adapter;
use super::tests::{SOURCE, fake_fc, frame, live_state, uplink_to};

/// A gimbal-scope control frame (overrides the flight-scope default).
fn gimbal_frame(
    axes: Vec<(LogicalAxisId, f32)>,
    edges: Vec<(LogicalButtonId, ButtonEdge)>,
) -> pilotage_protocol::ScopedControlFrame {
    let mut built = frame(axes, edges);
    built.scope = pilotage_protocol::ScopeId::new(super::GIMBAL_SCOPE);
    built
}

struct GimbalLanes {
    commands: tokio::sync::mpsc::Receiver<pilotage_mavlink::OutboundCommand>,
    rates: tokio::sync::watch::Receiver<Option<pilotage_mavlink::GimbalRateDemand>>,
}

fn gimbal_control() -> (crate::gimbal::Px4GimbalControl, GimbalLanes) {
    let (command_tx, command_rx) = tokio::sync::mpsc::channel(16);
    let (rate_tx, rate_rx) = tokio::sync::watch::channel(None);
    (
        crate::gimbal::Px4GimbalControl::new(command_tx, rate_tx, 1, 1),
        GimbalLanes {
            commands: command_rx,
            rates: rate_rx,
        },
    )
}

fn next_command(lanes: &mut GimbalLanes) -> u16 {
    lanes.commands.try_recv().expect("queued command").command
}

#[test]
fn capabilities_advertise_the_gimbal_scope_alongside_flight() {
    let (fc, addr) = fake_fc();
    drop(fc);
    let (control, _lanes) = gimbal_control();
    let adapter = Px4Adapter::from_state(VehicleId::new(1), live_state())
        .with_uplink(uplink_to(addr))
        .with_gimbal(control);
    let scopes: Vec<String> = adapter.capabilities().vehicles[0]
        .scopes
        .iter()
        .map(|descriptor| descriptor.scope.as_str().to_owned())
        .collect();
    assert_eq!(scopes, vec![super::FLIGHT_SCOPE, super::GIMBAL_SCOPE]);
}

#[test]
fn a_vehicle_without_a_gimbal_advertises_no_gimbal_scope() {
    let (fc, addr) = fake_fc();
    drop(fc);
    let adapter =
        Px4Adapter::from_state(VehicleId::new(1), live_state()).with_uplink(uplink_to(addr));
    let scopes: Vec<String> = adapter.capabilities().vehicles[0]
        .scopes
        .iter()
        .map(|descriptor| descriptor.scope.as_str().to_owned())
        .collect();
    assert_eq!(scopes, vec![super::FLIGHT_SCOPE], "no gimbal, no scope");
}

#[test]
fn gimbal_demands_flow_even_where_flight_control_cannot() {
    // A bare cache: no estimator authorization, so flight frames are
    // rejected — but pointing is not flight, and must keep working.
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (control, mut lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);

    let outcome = adapter.apply_control(&gimbal_frame(
        vec![
            (LogicalAxisId::new(super::PITCH_AXIS), 0.5),
            (LogicalAxisId::new(super::YAW_AXIS), -0.25),
        ],
        vec![],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert_eq!(
        next_command(&mut lanes),
        1001,
        "primary-control claim first"
    );
    let rate = *lanes.rates.borrow_and_update();
    assert!(
        rate.is_some(),
        "then the rate demand on the latest-value lane"
    );
}

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
        .expect("a gimbal-scope engage never actuates the FC, so it cannot fail");

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

#[test]
fn gimbal_neutral_button_recentres() {
    let (control, mut lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![],
        vec![(
            LogicalButtonId::new(super::GIMBAL_NEUTRAL_BUTTON),
            ButtonEdge::Pressed,
        )],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert_eq!(
        next_command(&mut lanes),
        1001,
        "primary-control claim first"
    );
    assert_eq!(
        next_command(&mut lanes),
        1000,
        "DO_GIMBAL_MANAGER_PITCHYAW recenters"
    );
}

#[test]
fn gimbal_frame_with_flight_axes_is_rejected() {
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::ROLL_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UnknownAxis),
        "the gimbal scope accepts pitch/yaw only"
    );
}

#[test]
fn fresh_claim_denial_rejects_pointing_frames_loudly() {
    let state = live_state();
    apply_messages_at(
        &state,
        &[(
            SOURCE,
            FcMessage::CommandAck {
                command: 1001,
                result: 4,
                target_system: 255,
                target_component: 190,
            },
        )],
        0,
        0,
        Instant::now(),
    );
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert!(
        matches!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::Other(_))
        ),
        "PX4 ignores non-primary demands silently, so a cached denial \
         must reject loudly instead: {:?}",
        outcome.disposition
    );
}

#[test]
fn an_unrelated_ack_does_not_bury_a_gimbal_claim_denial() {
    // A denied CONFIGURE (1001), then a later, unrelated
    // SET_MESSAGE_INTERVAL (511) acceptance. The CONFIGURE verdict is
    // tracked separately, so the denial must still reject pointing
    // frames — otherwise ignored gimbal demands would report accepted.
    let state = live_state();
    let deny = |c: u16, r: u8| {
        (
            SOURCE,
            FcMessage::CommandAck {
                command: c,
                result: r,
                target_system: 255,
                target_component: 190,
            },
        )
    };
    apply_messages_at(&state, &[deny(1001, 4)], 0, 0, Instant::now());
    apply_messages_at(&state, &[deny(511, 0)], 0, 0, Instant::now());
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert!(
        matches!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::Other(_))
        ),
        "the 511 ack must not bury the 1001 denial: {:?}",
        outcome.disposition
    );
}

#[test]
fn an_ack_addressed_to_another_endpoint_is_ignored() {
    // A CONFIGURE denial addressed to a different component proves
    // nothing about our claim; it must not reject our pointing frames.
    let state = live_state();
    apply_messages_at(
        &state,
        &[(
            SOURCE,
            FcMessage::CommandAck {
                command: 1001,
                result: 4,
                target_system: 42,
                target_component: 200,
            },
        )],
        0,
        0,
        Instant::now(),
    );
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(
        outcome.disposition,
        Disposition::Accepted,
        "an ack for another endpoint must not reject our frames"
    );
}

/// Feeds a gimbal-device attitude status into the shared cache at the
/// given receive instant, the way a live PX4 stream populates it.
fn feed_gimbal(state: &Arc<Mutex<LinkState>>, time_boot_ms: u32, failure_flags: u32, now: Instant) {
    apply_messages_at(
        state,
        &[(
            SOURCE,
            FcMessage::GimbalDeviceAttitudeStatus {
                time_boot_ms,
                quat_wxyz: [0.98, 0.0, -0.19, 0.0],
                rates_rps: [0.0, 0.05, -0.02],
                flags: 12,
                failure_flags,
            },
        )],
        0,
        0,
        now,
    );
}

#[test]
fn gimbal_attitude_is_stamped_as_a_payload_device_on_the_device_clock() {
    use pilotage_adapter_api::{MeasurementClock, SourceRole};
    let state = live_state();
    feed_gimbal(&state, 5_000, 0, Instant::now());
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let batch = adapter.sample_telemetry();
    let gimbal = batch.samples[0].gimbal.expect("gimbal sample present");
    assert_eq!(
        gimbal.stamp.role,
        SourceRole::PayloadDevice,
        "not a camera frame"
    );
    assert_eq!(
        gimbal.stamp.clock,
        MeasurementClock::VehicleBoot,
        "device boot clock"
    );
    // The device's own boot-relative time (ms) carried as ns, not host time.
    assert_eq!(gimbal.stamp.acquired_at_ns, 5_000 * 1_000_000);
    assert_eq!(gimbal.flags, 12);
    assert_eq!(gimbal.failure_flags, 0);
}

#[test]
fn gimbal_only_telemetry_survives_without_an_avionics_group() {
    // A cache with a gimbal report but no attitude/kinematics: the batch
    // would otherwise be empty and the gimbal would vanish.
    let state = Arc::new(Mutex::new(LinkState::default()));
    feed_gimbal(&state, 7_000, 0, Instant::now());
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let batch = adapter.sample_telemetry();
    assert_eq!(
        batch.samples.len(),
        1,
        "a carrier sample exists for gimbal-only"
    );
    assert!(batch.samples[0].avionics.is_none());
    assert!(
        batch.samples[0].gimbal.is_some(),
        "gimbal-only telemetry must reach clients"
    );
}

#[test]
fn a_gimbal_failure_flag_reaches_telemetry() {
    let state = live_state();
    feed_gimbal(&state, 5_000, 0b10, Instant::now());
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state).with_gimbal(control);
    let batch = adapter.sample_telemetry();
    let gimbal = batch.samples[0].gimbal.expect("gimbal sample present");
    assert_eq!(
        gimbal.failure_flags, 0b10,
        "device failure is surfaced, not dropped"
    );
}

/// A gimbal (or FC) reboot regresses the device's `time_boot_ms`. The
/// stamp must open a NEW epoch under a stable identity so acquisition
/// time never runs backwards within one (identity, epoch) — otherwise a
/// reboot is indistinguishable from a stale replay.
#[test]
fn a_gimbal_reboot_opens_a_new_epoch_instead_of_regressing_time() {
    let state = live_state();
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_gimbal(control);

    let stamp_now = |adapter: &mut Px4Adapter| {
        adapter.sample_telemetry().samples[0]
            .gimbal
            .expect("gimbal sample present")
            .stamp
    };

    // One boot session: two reports with rising device boot time.
    feed_gimbal(&state, 5_000, 0, Instant::now());
    let first = stamp_now(&mut adapter);
    feed_gimbal(&state, 6_000, 0, Instant::now());
    let later = stamp_now(&mut adapter);
    assert_eq!(
        first.source_epoch, later.source_epoch,
        "no reboot within one boot session"
    );
    assert!(
        later.acquired_at_ns > first.acquired_at_ns,
        "acquisition time advances within an epoch"
    );

    // Reboot: the device's boot time regresses 6000 -> 10 ms.
    feed_gimbal(&state, 10, 0, Instant::now());
    let after = stamp_now(&mut adapter);
    assert_eq!(
        after.source_epoch,
        later.source_epoch.wrapping_add(1),
        "a reboot opens the next epoch"
    );
    assert_eq!(
        after.source_incarnation, later.source_incarnation,
        "the same gimbal source identity spans the reboot; only the epoch turns over"
    );
    // The small post-reboot boot time lives under the NEW epoch, so no
    // consumer ever sees time regress within one (identity, epoch).

    // Re-sampling the same post-reboot report is not a second reboot.
    let resample = stamp_now(&mut adapter);
    assert_eq!(
        resample.source_epoch, after.source_epoch,
        "re-observing the same boot time keeps the epoch"
    );
}
