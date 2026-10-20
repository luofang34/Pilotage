use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{AttitudeTargetKind, FcMessage, FrameSource};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{AuthorizationSource, LandedState, LinkState};
use pilotage_protocol::{
    ControlAction, ControlPayload, Generation, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VehicleId,
};
use pilotage_timing::MonoTimestamp;

use super::{Phase, Reason};
use crate::uplink::tests::uplink;
use crate::uplink::{DirectOutcome, VelocityOutcome};

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn observed(now: Instant) -> Arc<Mutex<LinkState>> {
    let state = Arc::new(Mutex::new(LinkState {
        authorization_source: AuthorizationSource::ArduPilotEkfStatus,
        maximum_inter_group_skew_ms: 300,
        standard_status_max_lag_ms: 300,
        ..LinkState::default()
    }));
    feed(
        &state,
        &[
            FcMessage::Heartbeat {
                armed: true,
                custom_mode: 4,
                autopilot: 3,
            },
            FcMessage::AttitudeQuaternion {
                time_boot_ms: 1000,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
            FcMessage::EkfStatusReport { flags: 63 },
            FcMessage::AttitudeQuaternion {
                time_boot_ms: 1010,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
            FcMessage::LocalPositionNed {
                time_boot_ms: 1010,
                pos_ned_m: [0.0, 0.0, -7.0],
                vel_ned_mps: [0.0; 3],
            },
            FcMessage::RelativeAltitude {
                time_boot_ms: 1010,
                altitude_mm: 4000,
            },
            FcMessage::LandedState(LandedState::OnGround),
        ],
        now,
    );
    state
}

fn feed(state: &Arc<Mutex<LinkState>>, messages: &[FcMessage], now: Instant) {
    let tagged: Vec<_> = messages.iter().map(|message| (SOURCE, *message)).collect();
    apply_messages_at(state, &tagged, 0, 0, now);
}

fn frame() -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        session: SessionId::new(3),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(4),
        sequence: SequenceNum::new(5),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: None,
        actions: vec![ControlAction::Takeoff],
        action_ids: vec![6],
    }
}

#[test]
fn explicit_takeoff_queues_home_relative_target_and_preserves_request() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    let command = lanes.commands.try_recv().unwrap();
    assert_eq!(command.command, super::MAV_CMD_NAV_TAKEOFF);
    assert_eq!(command.params[6], 6.5);
    assert!(command.params[3].is_nan());
    assert!(command.fence.unwrap().is_valid());
    assert!(command.receipt.unwrap().sent_at_cached().is_none());
    assert_eq!(command.send_before, Some(now + Duration::from_millis(250)));
    let status = uplink.takeoff_status_cached().unwrap();
    assert_eq!(status.request.action_id, 6);
    assert_eq!(status.request.generation, Generation::new(4));
    assert_eq!(status.phase, Phase::Queued);
    let revision = status.revision;
    uplink.observe_takeoff(&state.lock().unwrap(), now);
    assert_eq!(uplink.takeoff_status_cached().unwrap().revision, revision);
}

#[test]
fn queued_takeoff_blocks_both_control_families_and_cannot_repeat() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, _lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    assert!(
        uplink
            .start_takeoff(&frame(), &state.lock().unwrap(), now)
            .is_err()
    );
    assert_eq!(
        uplink.apply_velocity([0.0; 4], false, true),
        VelocityOutcome::TakeoffInProgress
    );
    assert_eq!(
        uplink.apply_direct(AttitudeTargetKind::BodyRates([0.0; 3]), 0.5, true, 0.4, now),
        DirectOutcome::TakeoffBlocked
    );
}

#[test]
fn expired_write_never_opens_control_even_after_arm_or_disarmed_argument() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    let command = lanes.commands.try_recv().unwrap();
    uplink.observe_takeoff(&state.lock().unwrap(), now + Duration::from_millis(250));
    assert_eq!(
        uplink.takeoff_status_cached().unwrap().phase,
        Phase::TimedOut
    );
    assert!(!command.fence.unwrap().is_valid());
    uplink.observe_armed(false);
    uplink.arm(true, now);
    assert_eq!(
        uplink.apply_velocity([0.0; 4], false, true),
        VelocityOutcome::TakeoffBlocked
    );
    assert_eq!(
        uplink.apply_direct(AttitudeTargetKind::BodyRates([0.0; 3]), 0.5, true, 0.4, now),
        DirectOutcome::TakeoffBlocked
    );
}

#[test]
fn revocation_fences_queued_launch_and_only_fresh_disarm_recovers() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    let command = lanes.commands.try_recv().unwrap();
    uplink.cancel_takeoff(now);
    assert!(!command.fence.unwrap().is_valid());
    assert_eq!(
        uplink.takeoff_status_cached().unwrap().phase,
        Phase::Cancelled
    );
    assert_eq!(
        uplink.takeoff_status_cached().unwrap().reason,
        Some(Reason::AuthorityLost)
    );
    assert!(uplink.takeoff_blocks_motion());
    feed(
        &state,
        &[FcMessage::Heartbeat {
            armed: false,
            custom_mode: 4,
            autopilot: 3,
        }],
        now,
    );
    uplink.observe_takeoff(&state.lock().unwrap(), now);
    assert!(
        uplink.takeoff_blocks_motion(),
        "pre-request disarm cannot recover"
    );
    let later = now + Duration::from_millis(1);
    feed(
        &state,
        &[FcMessage::Heartbeat {
            armed: false,
            custom_mode: 4,
            autopilot: 3,
        }],
        later,
    );
    uplink.observe_takeoff(&state.lock().unwrap(), later);
    assert!(!uplink.takeoff_blocks_motion());
}

#[test]
fn missing_or_stale_flight_report_and_refused_queue_never_start() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    feed(
        &state,
        &[FcMessage::LandedState(LandedState::Undefined)],
        now,
    );
    assert!(
        uplink
            .start_takeoff(&frame(), &state.lock().unwrap(), now)
            .is_err()
    );
    feed(
        &state,
        &[FcMessage::LandedState(LandedState::OnGround)],
        now,
    );
    assert!(
        uplink
            .start_takeoff(
                &frame(),
                &state.lock().unwrap(),
                now + Duration::from_secs(4)
            )
            .is_err()
    );
    drop(lanes.commands);
    assert!(
        uplink
            .start_takeoff(&frame(), &state.lock().unwrap(), now)
            .is_err()
    );
    assert!(uplink.takeoff_status_cached().is_none());
    assert_eq!(uplink.send_failures(), 1);
}

#[test]
fn wrong_mode_or_disarm_precondition_cannot_queue_launch() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    for (armed, custom_mode) in [(true, 9), (false, 4)] {
        feed(
            &state,
            &[FcMessage::Heartbeat {
                armed,
                custom_mode,
                autopilot: 3,
            }],
            now,
        );
        assert!(
            uplink
                .start_takeoff(&frame(), &state.lock().unwrap(), now)
                .is_err()
        );
        assert!(lanes.commands.try_recv().is_err());
    }
}

#[test]
fn mode_loss_ends_tracking_and_does_not_open_motion() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    let command = lanes.commands.try_recv().unwrap();
    feed(
        &state,
        &[FcMessage::Heartbeat {
            armed: true,
            custom_mode: 9,
            autopilot: 3,
        }],
        now,
    );
    uplink.observe_takeoff(&state.lock().unwrap(), now);
    let status = uplink.takeoff_status_cached().unwrap();
    assert_eq!(status.phase, Phase::Failed);
    assert_eq!(status.reason, Some(Reason::ModeChanged));
    assert!(!command.fence.unwrap().is_valid());
    assert!(uplink.takeoff_blocks_motion());
}

#[test]
fn zero_remaining_budget_refuses_before_queueing() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    let mut command = frame();
    command.admitted_timing = Some(pilotage_protocol::AdmittedTiming {
        admitted_at: MonoTimestamp::from_nanos(0),
        send_remaining: Duration::ZERO,
        active_remaining: None,
        completion_timeout: Some(Duration::from_secs(5)),
        window_bounded: true,
    });
    assert!(
        uplink
            .start_takeoff(&command, &state.lock().unwrap(), now)
            .is_err()
    );
    assert!(lanes.commands.try_recv().is_err());
}

#[test]
fn changing_home_reference_without_local_climb_fails_closed() {
    let now = Instant::now();
    let state = observed(now);
    let (mut uplink, mut lanes) = uplink();
    uplink.bind_flight_state(state.clone());
    uplink
        .start_takeoff(&frame(), &state.lock().unwrap(), now)
        .unwrap();
    let command = lanes.commands.try_recv().unwrap();
    feed(
        &state,
        &[FcMessage::RelativeAltitude {
            time_boot_ms: 1020,
            altitude_mm: 7000,
        }],
        now,
    );
    uplink.observe_takeoff(&state.lock().unwrap(), now);
    let status = uplink.takeoff_status_cached().unwrap();
    assert_eq!(status.phase, Phase::Failed);
    assert_eq!(status.reason, Some(Reason::MeasurementUnavailable));
    assert!(!command.fence.unwrap().is_valid());
    assert_eq!(
        uplink.apply_velocity([0.0; 4], false, true),
        VelocityOutcome::TakeoffBlocked
    );
}
