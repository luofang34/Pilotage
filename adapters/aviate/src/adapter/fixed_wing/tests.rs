use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, IntentCapability, RejectReason, SourceIncarnation, VehicleAdapter,
};
use pilotage_control_feel::{
    FixedWingEnvelope, FlightFeelProfile, ValidatedFixedWingEnvelope, ValidatedFlightFeelProfile,
};
use pilotage_mavlink::codec::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, COMMAND_LONG_ID, HEARTBEAT_ID,
    LOCAL_POSITION_NED_ID, MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_CHANGE_SPEED,
    SET_POSITION_TARGET_ID, encode_frame_v2_from,
};
use pilotage_mavlink::link::LinkConfig;
use pilotage_protocol::{
    AltitudeReference, ControlAction, ControlIntent, HeadingAltitudeIntent, IntentFamily,
    ReferenceFrame, ScopedControlFrame, VehicleId, VelocityIntent,
};
use web_time::Instant;

use crate::adapter::{AviateAdapter, AviateAirframe, EmbeddedIncarnations, FLIGHT_SCOPE};

/// A host time far from the process clock, so freshness read from any
/// clock but the host's would fail.
fn host_start() -> Instant {
    Instant::now() + Duration::from_secs(50_000)
}

fn embedded_as(airframe: AviateAirframe, start: Instant) -> AviateAdapter {
    let incarnations = EmbeddedIncarnations {
        estimate: SourceIncarnation::new([1; 16]),
        fc_state: SourceIncarnation::new([2; 16]),
    };
    AviateAdapter::embedded_for(
        VehicleId::new(1),
        LinkConfig::default(),
        airframe,
        incarnations,
        start,
    )
    .expect("an embedded adapter")
}

fn multirotor(start: Instant) -> AviateAdapter {
    let feel = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("the legacy profile validates");
    embedded_as(AviateAirframe::Multirotor(Box::new(feel)), start)
}

fn fixed_wing(start: Instant) -> AviateAdapter {
    let c172 = ValidatedFixedWingEnvelope::new(FixedWingEnvelope::C172).expect("valid");
    embedded_as(AviateAirframe::FixedWing(c172), start)
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

fn velocity(vx: f32, vz: f32) -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz,
        yaw_rate: 0.0,
    })
}

/// A local position and velocity at `time_boot_ms`, beside the attitude:
/// the full pose a velocity demand needs.
fn send_kinematics(adapter: &mut AviateAdapter, time_boot_ms: u32) {
    let mut kinematics = Vec::new();
    kinematics.extend_from_slice(&time_boot_ms.to_le_bytes());
    for value in [0.0_f32, 0.0, -300.0, 55.0, 0.0, 0.0] {
        kinematics.extend_from_slice(&value.to_le_bytes());
    }
    adapter.on_downlink(&frame_of(LOCAL_POSITION_NED_ID, 185, &kinematics));
}

/// A heartbeat on the command link from an FC of `mav_type`.
fn heartbeat_from(adapter: &mut AviateAdapter, mav_type: u8) {
    let payload = [0, 0, 0, 0, mav_type, 12, 0x01, 4, 3];
    assert!(adapter.on_uplink_reply(&frame_of(HEARTBEAT_ID, 50, &payload)));
    adapter.sample_telemetry();
}

/// The NED velocity of a SET_POSITION_TARGET_LOCAL_NED frame.
fn velocity_of(frame: &[u8]) -> [f32; 3] {
    let at = |offset: usize| {
        f32::from_le_bytes(
            frame[10 + offset..14 + offset]
                .try_into()
                .expect("four bytes"),
        )
    };
    [at(16), at(20), at(24)]
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
    let outcome = adapter.apply_control(&frame_with(Some(velocity(55.0, 0.0)), vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    let arm_with_velocity = frame_with(Some(velocity(55.0, 0.0)), vec![ControlAction::Arm]);
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
fn a_fixed_wing_advertises_its_class_and_envelope_and_no_stick_law() {
    let advertised = |adapter: &AviateAdapter, family| -> Option<IntentCapability> {
        adapter.capabilities().vehicles[0]
            .scopes
            .iter()
            .flat_map(|scope| scope.intents.iter())
            .find(|intent| intent.family == family)
            .cloned()
    };
    let c172 = fixed_wing(host_start());
    let capabilities = c172.capabilities();
    assert_eq!(
        capabilities.vehicles[0].class,
        pilotage_protocol::VehicleClass::FixedWing
    );
    assert!(capabilities.control_feel.is_none(), "no Alia law");
    assert!(
        capabilities.vehicles[0]
            .scopes
            .iter()
            .all(|scope| scope.scope.as_str() != crate::adapter::DIRECT_SCOPE),
        "no direct attitude scope"
    );
    let velocity = advertised(&c172, IntentFamily::Velocity).expect("a velocity");
    assert_eq!(velocity.max_linear, 65.0);
    assert_eq!(velocity.max_vertical, 5.0);
    assert!(velocity.max_angular > 0.0);
    assert_eq!(
        advertised(&c172, IntentFamily::HeadingAltitude).map(|intent| intent.max_linear),
        Some(65.0)
    );

    let quad = multirotor(host_start());
    assert_eq!(
        quad.capabilities().vehicles[0].class,
        pilotage_protocol::VehicleClass::Multicopter
    );
    assert_eq!(advertised(&quad, IntentFamily::HeadingAltitude), None);
    assert_eq!(
        advertised(&quad, IntentFamily::Velocity).map(|intent| intent.max_linear),
        Some(3.0),
        "the multirotor keeps its law"
    );
}

#[test]
fn a_level_cruise_goes_out_after_the_arm_with_no_climb_input() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut adapter, 1_000);
    send_kinematics(&mut adapter, 1_000);
    let arm = adapter.apply_control(&frame_with(
        Some(velocity(55.0, 0.0)),
        vec![ControlAction::Arm],
    ));
    assert_eq!(arm.disposition, Disposition::Accepted);
    adapter.drain_uplink();
    adapter.set_now(start + Duration::from_millis(1_300));
    send_attitude_only(&mut adapter, 1_300);
    send_kinematics(&mut adapter, 1_300);
    let cruise = adapter.apply_control(&frame_with(Some(velocity(55.0, 0.0)), vec![]));
    assert_eq!(cruise.disposition, Disposition::Accepted);
    let sent = adapter.drain_uplink();
    assert_eq!(message_ids(&sent), vec![SET_POSITION_TARGET_ID]);
    assert_eq!(velocity_of(&sent[0]), [55.0, 0.0, 0.0], "north at yaw 0");
}

#[test]
fn a_demand_outside_the_envelope_is_bounded_and_reported_constrained() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut adapter, 1_000);
    send_kinematics(&mut adapter, 1_000);
    for (demand, expected) in [
        (velocity(90.0, -9.0), [65.0, 0.0, -2.5]),
        (velocity(10.0, 9.0), [33.0, 0.0, 5.0]),
    ] {
        let outcome = adapter.apply_control(&frame_with(Some(demand), vec![]));
        assert_eq!(outcome.disposition, Disposition::Constrained);
        let sent = adapter.drain_uplink();
        assert_eq!(velocity_of(&sent[0]), expected);
    }
    let level = adapter.apply_control(&frame_with(Some(velocity(0.0, -2.0)), vec![]));
    assert_eq!(
        level.disposition,
        Disposition::Accepted,
        "no direction: the FC holds the heading at cruise"
    );
    assert_eq!(velocity_of(&adapter.drain_uplink()[0]), [0.0, 0.0, -2.0]);
}

#[test]
fn an_fc_that_reports_another_airframe_is_refused_and_disarm_still_goes() {
    let start = host_start();
    let mut c172 = fixed_wing(start);
    c172.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut c172, 1_000);
    heartbeat_from(&mut c172, 0);
    assert_eq!(
        c172.apply_control(&frame_with(None, vec![ControlAction::Arm]))
            .disposition,
        Disposition::Accepted,
        "a generic type contradicts nothing"
    );
    c172.drain_uplink();
    heartbeat_from(&mut c172, 2);
    assert_eq!(
        c172.apply_control(&frame_with(None, vec![ControlAction::Arm]))
            .disposition,
        Disposition::Rejected(RejectReason::AirframeMismatch {
            reported_mav_type: 2
        })
    );
    assert!(c172.drain_uplink().is_empty());
    assert_eq!(
        c172.apply_control(&frame_with(None, vec![ControlAction::Disarm]))
            .disposition,
        Disposition::Accepted,
        "surrendering authority is never blocked"
    );
    heartbeat_from(&mut c172, 1);
    assert_eq!(
        c172.apply_control(&frame_with(None, vec![ControlAction::Arm]))
            .disposition,
        Disposition::Accepted
    );

    let mut quad = multirotor(start);
    quad.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut quad, 1_000);
    heartbeat_from(&mut quad, 1);
    assert_eq!(
        quad.apply_control(&frame_with(None, vec![ControlAction::Arm]))
            .disposition,
        Disposition::Rejected(RejectReason::AirframeMismatch {
            reported_mav_type: 1
        })
    );
}

#[test]
fn a_multirotor_neither_advertises_nor_flies_a_heading_with_an_altitude() {
    let start = host_start();
    let mut quad = multirotor(start);
    quad.set_now(start + Duration::from_secs(1));
    send_attitude_only(&mut quad, 1_000);
    assert!(matches!(
        quad.apply_control(&frame_with(
            Some(heading_altitude(AltitudeReference::LocalOrigin, None)),
            vec![],
        ))
        .disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
}

#[test]
fn an_origin_height_that_is_not_a_number_is_refused() {
    let mut adapter = fixed_wing(host_start());
    assert!(adapter.set_local_origin_height(f64::INFINITY).is_err());
    assert_eq!(adapter.fixed_wing.origin_ellipsoid_height_m, None);
}

#[test]
fn the_fc_state_says_whether_the_fc_would_accept_an_arm() {
    let start = host_start();
    let mut adapter = fixed_wing(start);
    adapter.set_now(start + Duration::from_millis(200));
    let ready = |adapter: &mut AviateAdapter, system_status: u8| {
        let payload = [0, 0, 0, 0, 1, 12, 0x01, system_status, 3];
        assert!(adapter.on_uplink_reply(&frame_of(HEARTBEAT_ID, 50, &payload)));
        adapter
            .sample_telemetry()
            .samples
            .first()
            .and_then(|sample| sample.fc_state)
            .map(|fc_state| fc_state.ready_to_arm)
    };
    assert_eq!(ready(&mut adapter, 1), Some(Some(false)), "booting");
    assert_eq!(ready(&mut adapter, 3), Some(Some(true)), "standing by");
    assert_eq!(ready(&mut adapter, 0), Some(None), "uninitialized");
}
