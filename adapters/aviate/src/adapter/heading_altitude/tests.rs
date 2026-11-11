use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, IntentCapability, RejectReason, SourceIncarnation, VehicleAdapter,
};
use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_mavlink::codec::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, COMMAND_LONG_ID,
    MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_CHANGE_SPEED, SET_POSITION_TARGET_ID,
    encode_frame_v2_from,
};
use pilotage_mavlink::link::LinkConfig;
use pilotage_protocol::{
    AltitudeReference, ControlAction, ControlIntent, HeadingAltitudeIntent, IntentFamily,
    ReferenceFrame, ScopedControlFrame, VehicleId, VelocityIntent,
};
use web_time::Instant;

use crate::adapter::{AviateAdapter, EmbeddedIncarnations, FLIGHT_SCOPE};

/// A host time far from the process clock, so freshness read from any
/// clock but the host's would fail.
fn host_start() -> Instant {
    Instant::now() + Duration::from_secs(50_000)
}

fn embedded(start: Instant) -> AviateAdapter {
    let feel = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("the legacy profile validates");
    let incarnations = EmbeddedIncarnations {
        estimate: SourceIncarnation::new([1; 16]),
        fc_state: SourceIncarnation::new([2; 16]),
    };
    AviateAdapter::embedded(
        VehicleId::new(1),
        LinkConfig::default(),
        feel,
        incarnations,
        start,
    )
    .expect("an embedded adapter")
}

fn frame_of(msg_id: u32, extra: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0_u8; 10 + payload.len() + 2];
    let written = encode_frame_v2_from(0, msg_id, payload, extra, 1, 1, &mut frame);
    frame.truncate(written);
    frame
}

/// An authorized estimator status, then an attitude at `time_boot_ms`:
/// what a flight controller without a position estimate still sends.
fn send_attitude_only(adapter: &mut AviateAdapter, time_boot_ms: u32) {
    let mut status = Vec::new();
    status.extend_from_slice(&(u64::from(time_boot_ms) * 1_000).to_le_bytes());
    // Every group valid, and the wire quality the link reads as good.
    status.extend_from_slice(&[0x0f, 2]);
    adapter.on_downlink(&frame_of(AVIATE_ESTIMATOR_STATUS_ID, 171, &status));
    let mut attitude = Vec::new();
    attitude.extend_from_slice(&time_boot_ms.to_le_bytes());
    for value in [1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] {
        attitude.extend_from_slice(&value.to_le_bytes());
    }
    adapter.on_downlink(&frame_of(ATTITUDE_QUATERNION_ID, 246, &attitude));
    let state = adapter.estimate.as_ref().expect("an estimate link");
    let latest = state.state.lock().expect("lock");
    assert!(latest.attitude.is_some(), "the attitude was accepted");
    assert!(latest.kinematics.is_none(), "no position estimate");
}

fn heading_altitude(reference: AltitudeReference, airspeed_mps: Option<f32>) -> ControlIntent {
    ControlIntent::HeadingAltitude(HeadingAltitudeIntent {
        heading_rad: 0.8,
        altitude_m: 2_000.0,
        reference,
        airspeed_mps,
    })
}

fn frame_with(intent: Option<ControlIntent>, actions: Vec<ControlAction>) -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        action_ids: vec![],
        session: pilotage_protocol::SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: pilotage_protocol::ScopeId::new(FLIGHT_SCOPE),
        generation: pilotage_protocol::Generation::new(1),
        sequence: pilotage_protocol::SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: pilotage_protocol::ControlPayload::default(),
        intent,
        actions,
    }
}

fn velocity() -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    })
}

fn message_ids(frames: &[Vec<u8>]) -> Vec<u32> {
    frames
        .iter()
        .map(|frame| u32::from(frame[7]) | (u32::from(frame[8]) << 8))
        .collect()
}

fn command_of(frame: &[u8]) -> u16 {
    u16::from_le_bytes([frame[10 + 28], frame[10 + 29]])
}

/// The down coordinate of a SET_POSITION_TARGET_LOCAL_NED frame.
fn down_of(frame: &[u8]) -> f32 {
    f32::from_le_bytes(frame[22..26].try_into().expect("four bytes"))
}

fn fixed_wing(start: Instant) -> AviateAdapter {
    let mut adapter = embedded(start);
    adapter.enable_heading_altitude(70.0).expect("a bound");
    adapter
}

#[test]
fn an_arm_and_a_heading_with_an_altitude_fly_on_attitude_alone() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_local_origin_height(431.0).expect("a height");
    adapter.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut adapter, 1_000);

    let arm = adapter.apply_control(&frame_with(None, vec![ControlAction::Arm]));
    assert_eq!(arm.disposition, Disposition::Accepted);
    let sent = adapter.drain_uplink();
    assert_eq!(message_ids(&sent), vec![COMMAND_LONG_ID]);
    assert_eq!(command_of(&sent[0]), MAV_CMD_COMPONENT_ARM_DISARM);

    // Past the post-arm quiet interval.
    adapter.set_now(start + Duration::from_millis(1_300));
    send_attitude_only(&mut adapter, 1_300);
    let outcome = adapter.apply_control(&frame_with(
        Some(heading_altitude(AltitudeReference::Ellipsoid, Some(55.0))),
        vec![],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let sent = adapter.drain_uplink();
    assert_eq!(
        message_ids(&sent),
        vec![COMMAND_LONG_ID, SET_POSITION_TARGET_ID]
    );
    assert_eq!(command_of(&sent[0]), MAV_CMD_DO_CHANGE_SPEED);
    assert_eq!(down_of(&sent[1]), -(2_000.0 - 431.0));
}

#[test]
fn a_velocity_still_needs_the_full_pose() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut adapter, 1_000);
    let outcome = adapter.apply_control(&frame_with(Some(velocity()), vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    let arm_with_velocity = frame_with(Some(velocity()), vec![ControlAction::Arm]);
    assert_eq!(
        adapter.apply_control(&arm_with_velocity).disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable),
        "an arm riding a velocity keeps the velocity's gate"
    );
    assert!(adapter.drain_uplink().is_empty());
}

#[test]
fn an_ellipsoidal_altitude_without_the_origin_height_is_refused_and_a_local_one_flies() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut adapter, 1_000);
    let refused = adapter.apply_control(&frame_with(
        Some(heading_altitude(AltitudeReference::Ellipsoid, None)),
        vec![ControlAction::Arm],
    ));
    assert_eq!(
        refused.disposition,
        Disposition::Rejected(RejectReason::LocalOriginHeightUnknown)
    );
    assert!(
        adapter.drain_uplink().is_empty(),
        "no arm goes out beside a refused demand"
    );
    let local = adapter.apply_control(&frame_with(
        Some(heading_altitude(AltitudeReference::LocalOrigin, None)),
        vec![],
    ));
    assert_eq!(local.disposition, Disposition::Accepted);
    let sent = adapter.drain_uplink();
    assert_eq!(message_ids(&sent), vec![SET_POSITION_TARGET_ID]);
    assert_eq!(down_of(&sent[0]), -2_000.0);
}

#[test]
fn only_an_enabled_vehicle_advertises_and_flies_the_intent() {
    let start = host_start();
    let mut multirotor = embedded(start);
    let advertised = |adapter: &AviateAdapter| -> Option<IntentCapability> {
        adapter.capabilities().vehicles[0]
            .scopes
            .iter()
            .flat_map(|scope| scope.intents.iter())
            .find(|intent| intent.family == IntentFamily::HeadingAltitude)
            .cloned()
    };
    assert_eq!(advertised(&multirotor), None);
    multirotor.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut multirotor, 1_000);
    assert!(matches!(
        multirotor
            .apply_control(&frame_with(
                Some(heading_altitude(AltitudeReference::LocalOrigin, None)),
                vec![],
            ))
            .disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
    let fixed_wing = fixed_wing(start);
    assert_eq!(
        advertised(&fixed_wing).map(|intent| intent.max_linear),
        Some(70.0)
    );
}

#[test]
fn settings_that_are_not_numbers_are_refused() {
    let mut adapter = embedded(host_start());
    assert!(adapter.enable_heading_altitude(0.0).is_err());
    assert!(adapter.enable_heading_altitude(f32::NAN).is_err());
    assert!(adapter.set_local_origin_height(f64::INFINITY).is_err());
    assert_eq!(
        adapter.heading_altitude,
        super::HeadingAltitudeSettings::default()
    );
}
