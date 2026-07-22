//! The direct-flight scope (CTRL-01 `vehicle.motion.direct`): attitude +
//! collective under its own lease — the adapter routes by SCOPE, never by a
//! latched mode that reinterprets velocity numbers.

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, IntentFamily, LogicalButtonId, VehicleId};

use super::super::{ARM_BUTTON, AviateAdapter, DIRECT_SCOPE};
use super::fixtures::{direct_frame, flight_frame, state_with};

fn field(buf: &[u8; 128], off: usize) -> f32 {
    f32::from_le_bytes([buf[10 + off], buf[11 + off], buf[12 + off], buf[13 + off]])
}

fn flying_adapter(fc: &std::net::UdpSocket) -> AviateAdapter {
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    uplink.use_manual_clock();
    let state = state_with(std::time::Duration::ZERO, std::time::Duration::ZERO);
    AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink)
}

fn tick_clock(adapter: &mut AviateAdapter, ms: u64) {
    adapter
        .uplink_mut()
        .expect("uplink bound")
        .advance_clock(std::time::Duration::from_millis(ms));
}

#[test]
fn the_direct_scope_is_advertised_with_attitude_thrust_and_no_mode_request() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let adapter = flying_adapter(&fc);
    let caps = adapter.capabilities();
    let direct = caps.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == DIRECT_SCOPE)
        .expect("direct scope advertised");
    assert_eq!(direct.intents.len(), 1);
    let intent = &direct.intents[0];
    assert_eq!(intent.family, IntentFamily::AttitudeThrust);
    assert!(intent.max_angular > 0.0, "tilt bound advertised");
    assert!(intent.max_yaw_rate > 0.0, "heading slew advertised");
    assert!(direct.legacy.is_none(), "typed-only: no legacy translation");
    for scope in &caps.vehicles[0].scopes {
        assert!(
            scope
                .actions
                .iter()
                .all(|action| action.action != pilotage_protocol::ActionKind::ModeRequest),
            "no mode requests: direct flight is its own scope"
        );
    }
}

#[test]
fn a_direct_frame_reaches_the_fc_as_an_attitude_setpoint() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .expect("timeout");
    let mut adapter = flying_adapter(&fc);
    let mut buf = [0u8; 128];

    // Arm on the velocity scope (both scopes share the discrete actions),
    // then wait out the post-arm quiet window.
    adapter.apply_control(&flight_frame(
        vec![],
        vec![(LogicalButtonId::new(ARM_BUTTON), ButtonEdge::Pressed)],
    ));
    fc.recv_from(&mut buf).expect("arm frame");
    tick_clock(&mut adapter, 200);

    // Climb collective opens the setpoint stream; tilt and heading pass
    // through the euler round-trip intact.
    let outcome = adapter.apply_control(&direct_frame(0.2, -0.1, 1.0, 0.9));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let (_, _) = fc.recv_from(&mut buf).expect("attitude frame");
    assert_eq!(buf[7], 82, "SET_ATTITUDE_TARGET id");
    // Recover the euler angles from the encoded quaternion at [4..20).
    let (qw, qx, qy, qz) = (
        field(&buf, 4),
        field(&buf, 8),
        field(&buf, 12),
        field(&buf, 16),
    );
    let roll = (2.0 * (qw * qx + qy * qz)).atan2(1.0 - 2.0 * (qx * qx + qy * qy));
    let pitch = (2.0 * (qw * qy - qz * qx)).asin();
    let yaw = (2.0 * (qw * qz + qx * qy)).atan2(1.0 - 2.0 * (qy * qy + qz * qz));
    assert!((roll - 0.2).abs() < 1e-3, "roll {roll}");
    assert!((pitch + 0.1).abs() < 1e-3, "pitch {pitch}");
    assert!((yaw - 1.0).abs() < 1e-3, "yaw {yaw}");
    // thrust 0.9 → stick 0.8 → hover-anchored collective above hover.
    let thrust = field(&buf, 32);
    assert!(thrust > 0.72 && thrust <= 1.0, "collective {thrust}");
}

#[test]
fn a_velocity_intent_on_the_direct_scope_is_rejected() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let mut adapter = flying_adapter(&fc);
    // The session gate blocks this before delivery; the adapter still
    // refuses defensively.
    let mut frame = flight_frame(vec![], vec![]);
    frame.scope = pilotage_protocol::ScopeId::new(DIRECT_SCOPE);
    let outcome = adapter.apply_control(&frame);
    assert!(
        matches!(outcome.disposition, Disposition::Rejected(_)),
        "got {:?}",
        outcome.disposition
    );
}

#[test]
fn a_hover_posture_attitude_satisfies_neutral_activation() {
    // The link-loss recovery path on the direct scope: level tilt at the
    // mid collective is the demonstrable neutral posture.
    let capability = pilotage_adapter_api::IntentCapability {
        family: IntentFamily::AttitudeThrust,
        frames: vec![pilotage_protocol::ReferenceFrame::LocalNed],
        max_linear: 0.0,
        max_vertical: 0.0,
        max_angular: 0.6,
        max_yaw_rate: 0.9,
    };
    let level = |thrust: f32, yaw: f32| {
        let frame = direct_frame(0.0, 0.0, yaw, thrust);
        frame.intent.expect("intent")
    };
    assert!(pilotage_adapter_api::intent_satisfies_neutral_activation(
        &level(0.5, 2.0),
        &capability,
        50,
    ));
    assert!(
        !pilotage_adapter_api::intent_satisfies_neutral_activation(
            &level(0.9, 0.0),
            &capability,
            50,
        ),
        "high collective is a climb demand, not neutral"
    );
    let tilted = direct_frame(0.3, 0.0, 0.0, 0.5).intent.expect("intent");
    assert!(
        !pilotage_adapter_api::intent_satisfies_neutral_activation(&tilted, &capability, 50),
        "a tilt demand is not neutral"
    );
}

/// The physical/RF profile: the lifecycle capability is STRUCTURALLY
/// absent — never advertised, never executed — even though the vehicle
/// has a live uplink (SIM-01).
#[test]
fn a_physical_profile_neither_advertises_nor_accepts_lifecycle_commands() {
    use super::super::AviateProfile;
    use super::fixtures::lifecycle_reset_frame;

    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let mut adapter = flying_adapter(&fc).with_profile(AviateProfile::Physical);

    let caps = adapter.capabilities();
    assert!(
        caps.vehicles[0]
            .scopes
            .iter()
            .all(|scope| scope.scope.as_str() != pilotage_adapter_api::SIM_LIFECYCLE_SCOPE),
        "a physical vehicle must not advertise the lifecycle scope"
    );
    // Both flight scopes remain: physical vehicles still fly.
    assert_eq!(caps.vehicles[0].scopes.len(), 2);

    // A forged lifecycle frame is refused whole and spawns nothing.
    let outcome = adapter.apply_control(&lifecycle_reset_frame());
    assert!(
        matches!(
            outcome.disposition,
            Disposition::Rejected(pilotage_adapter_api::RejectReason::Other(_))
        ),
        "got {:?}",
        outcome.disposition
    );
    assert_eq!(adapter.reset_spawns, 0, "the reset script never spawns");
}
