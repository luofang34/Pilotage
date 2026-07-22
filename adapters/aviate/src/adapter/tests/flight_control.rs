//! Flight-control uplink behavior through the adapter boundary: the
//! stick-to-setpoint path, the brake-then-hold state machine, and its
//! velocity-evidence rules, all against a fake FC socket and the
//! uplink's manual clock (no real-time sleeps).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, VehicleId};

use pilotage_mavlink::link::{KinematicsUpdate, LinkState};

use super::super::{ARM_BUTTON, AviateAdapter, PITCH_AXIS, THROTTLE_AXIS};
use super::fixtures::{flight_frame, state_with};

/// A little-endian f32 payload field at `off` bytes into the frame body.
fn field(buf: &[u8; 128], off: usize) -> f32 {
    f32::from_le_bytes([buf[10 + off], buf[11 + off], buf[12 + off], buf[13 + off]])
}

/// The SET_POSITION_TARGET type mask (velocity mode 2503, position 2552).
fn type_mask(buf: &[u8; 128]) -> u16 {
    u16::from_le_bytes([buf[10 + 48], buf[11 + 48]])
}

/// An all-centered stick frame.
fn centered_frame() -> pilotage_protocol::ScopedControlFrame {
    flight_frame(
        vec![
            (LogicalAxisId::new(PITCH_AXIS), 0.0),
            (LogicalAxisId::new(THROTTLE_AXIS), 0.0),
        ],
        vec![],
    )
}

/// Rewrites the fixture's kinematics group, fresh as of now.
fn set_kinematics(state: &Arc<Mutex<LinkState>>, mutate: impl FnOnce(&mut KinematicsUpdate)) {
    let mut latest = state.lock().expect("state lock");
    let kin = latest.kinematics.as_mut().expect("kinematics fixture");
    mutate(kin);
    kin.received_at = std::time::Instant::now();
}

/// Rewrites the fixture's measured velocity, fresh as of now.
fn set_velocity(state: &Arc<Mutex<LinkState>>, vel: [f32; 3]) {
    set_kinematics(state, |kin| kin.vel_ned_mps = vel);
}

/// An adapter wired to a fake FC (any UDP socket the uplink's frames
/// can be read from), heading east (yaw 90°) so body-frame rotation is
/// observable. The state handle steers the fixture's measurements; the
/// uplink runs on its manual clock so no test sleeps against real time.
fn flying_adapter(fc: &std::net::UdpSocket) -> (AviateAdapter, Arc<Mutex<LinkState>>) {
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    uplink.use_manual_clock();
    let state = state_with(Duration::ZERO, Duration::ZERO);
    let adapter = AviateAdapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink);
    (adapter, state)
}

/// Advances the uplink's manual clock by `ms` milliseconds.
fn tick_clock(adapter: &mut AviateAdapter, ms: u64) {
    adapter
        .uplink_mut()
        .expect("uplink bound")
        .advance_clock(Duration::from_millis(ms));
}

/// Arms, waits out the post-arm quiet window, then streams frames of
/// full-forward stick with half climb like the 30 Hz control loop,
/// asserting the slew limiter ramps the command from zero. Returns the
/// last east-velocity command, leaving the vehicle flying east.
fn arm_and_ramp_east(
    adapter: &mut AviateAdapter,
    fc: &std::net::UdpSocket,
    buf: &mut [u8; 128],
) -> f32 {
    // Arm edge (stick frames are suppressed for a beat afterward so the
    // FC's single command slot is not overwritten before its loop runs).
    let outcome = adapter.apply_control(&flight_frame(
        vec![],
        vec![(LogicalButtonId::new(ARM_BUTTON), ButtonEdge::Pressed)],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);

    // First datagram: the arm command (COMMAND_LONG 400, param1=1).
    let (n, _) = fc.recv_from(buf).expect("arm frame");
    assert_eq!(buf[7], 76, "COMMAND_LONG id");
    assert_eq!(field(buf, 0), 1.0);
    assert!(n >= 45);

    // Step the manual clock past the post-arm quiet window, then 20 ms
    // per frame like the 30 Hz control loop — no real-time sleeps.
    tick_clock(adapter, 200);
    let mut last_ve = 0.0f32;
    for _ in 0..30 {
        let outcome = adapter.apply_control(&flight_frame(
            vec![
                (LogicalAxisId::new(PITCH_AXIS), 1.0),
                (LogicalAxisId::new(THROTTLE_AXIS), 0.5),
            ],
            vec![],
        ));
        assert_eq!(outcome.disposition, Disposition::Accepted);
        fc.recv_from(buf).expect("setpoint frame");
        assert_eq!(buf[7], 84, "SET_POSITION_TARGET id");
        let ve = field(buf, 20);
        assert!(
            ve >= last_ve - 1e-4,
            "ramp must not reverse: {ve} < {last_ve}"
        );
        last_ve = ve;
        tick_clock(adapter, 20);
    }
    last_ve
}

#[test]
fn stick_frame_reaches_the_fc_as_a_velocity_setpoint() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (mut adapter, _state) = flying_adapter(&fc);

    let caps = adapter.capabilities();
    assert_eq!(
        caps.vehicles[0].scopes.len(),
        2,
        "velocity + direct flight scopes advertised"
    );
    assert_eq!(caps.vehicles[0].scopes[0].axes.len(), 4);

    let mut buf = [0u8; 128];
    let last_ve = arm_and_ramp_east(&mut adapter, &fc, &mut buf);

    // Heading is east (yaw 90°), stick full-forward → velocity is +east:
    // vn≈0, ve ramping toward 3.
    let (vn, vz) = (field(&buf, 16), field(&buf, 24));
    assert!(vn.abs() < 0.05, "vn {vn}");
    assert!(last_ve > 1.0, "ramped east velocity, got {last_ve}");
    assert!(vz < -0.2, "climb demand present, got {vz}");
    assert_eq!(type_mask(&buf), 2503);
}

#[test]
fn centered_sticks_brake_to_a_stop_before_capturing_the_hold() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (mut adapter, state) = flying_adapter(&fc);
    let mut buf = [0u8; 128];
    arm_and_ramp_east(&mut adapter, &fc, &mut buf);

    // Centered sticks while still moving (fake estimate: |vel| ≈ 5 m/s)
    // brake first: zero-velocity setpoints (mask 2503), NOT a hold
    // point captured at release — that point would be overrun and
    // flown back to.
    let outcome = adapter.apply_control(&centered_frame());
    assert_eq!(outcome.disposition, Disposition::Accepted);
    fc.recv_from(&mut buf).expect("brake frame");
    assert_eq!(type_mask(&buf), 2503, "braking streams velocity mode");
    for (off, axis) in [(16, "vn"), (20, "ve"), (24, "vz")] {
        let v = field(&buf, off);
        assert!(v.abs() < 1e-6, "brake demands zero {axis}, got {v}");
    }

    // A purely vertical residual (1 m/s climb) is still motion: the
    // brake phase covers the vertical axis, not just horizontal.
    set_velocity(&state, [0.0, 0.0, -1.0]);
    adapter.apply_control(&centered_frame());
    fc.recv_from(&mut buf).expect("vertical brake frame");
    assert_eq!(type_mask(&buf), 2503, "vertical residual keeps braking");

    // Braking finished (|vel| below the capture threshold): NOW the
    // hold point is captured and streamed as a position-valid setpoint
    // (mask 2552) — the fake pose's NED position, no ground-side gains.
    set_velocity(&state, [0.1, 0.05, 0.0]);
    let outcome = adapter.apply_control(&centered_frame());
    assert_eq!(outcome.disposition, Disposition::Accepted);
    fc.recv_from(&mut buf).expect("hold frame");
    assert_eq!(type_mask(&buf), 2552, "hold streams FC position mode");
    // Position fields carry the captured point (fake pose 10, 20, -30).
    assert!((field(&buf, 4) - 10.0).abs() < 1e-3, "hold north");
    assert!((field(&buf, 8) - 20.0).abs() < 1e-3, "hold east");
    assert!((field(&buf, 12) + 30.0).abs() < 1e-3, "hold down");

    // A gust (velocity back above threshold) must NOT drop the captured
    // hold point — re-capturing on a blip would walk the vehicle
    // downwind. The hold stays until a stick deflects.
    set_velocity(&state, [3.0, 4.0, -1.0]);
    adapter.apply_control(&centered_frame());
    fc.recv_from(&mut buf).expect("sticky hold frame");
    assert_eq!(
        type_mask(&buf),
        2552,
        "captured hold survives a velocity blip"
    );
    assert!((field(&buf, 4) - 10.0).abs() < 1e-3, "sticky hold north");
}

/// Asserts the next frame the fake FC receives is a zero-demand
/// velocity-mode setpoint (the braking frame).
fn expect_brake_frame(fc: &std::net::UdpSocket, buf: &mut [u8; 128], why: &str) {
    fc.recv_from(buf).expect(why);
    assert_eq!(type_mask(buf), 2503, "{why}: braking streams velocity mode");
    for off in [16, 20, 24] {
        assert!(field(buf, off).abs() < 1e-6, "{why}: zero demand");
    }
}

#[test]
fn invalid_velocity_keeps_braking_and_never_captures_the_hold() {
    // Position-valid but velocity-invalid: the estimate withholds the
    // velocity group (bit 3 cleared), so stillness can never be
    // demonstrated — the uplink must keep braking, not capture a hold
    // from absent data.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (mut adapter, state) = flying_adapter(&fc);
    let mut buf = [0u8; 128];
    arm_and_ramp_east(&mut adapter, &fc, &mut buf);

    // Even a numerically tiny velocity is no evidence while undeclared.
    set_kinematics(&state, |kin| {
        kin.vel_ned_mps = [0.0, 0.0, 0.0];
        kin.valid_flags = 0b0111;
    });
    for _ in 0..3 {
        adapter.apply_control(&centered_frame());
        expect_brake_frame(&fc, &mut buf, "velocity-invalid frame");
    }

    // Validity restored with demonstrated stillness: NOW the hold captures.
    set_kinematics(&state, |kin| {
        kin.vel_ned_mps = [0.1, 0.0, 0.0];
        kin.valid_flags = 0b1111;
    });
    adapter.apply_control(&centered_frame());
    fc.recv_from(&mut buf).expect("hold frame");
    assert_eq!(
        type_mask(&buf),
        2552,
        "validated stillness captures the hold"
    );
}

#[test]
fn non_finite_velocity_keeps_braking() {
    // NaN compares false against any threshold; without independent
    // validation that silently reads as "stopped" and captures the hold
    // at full speed — the exact defect brake-then-hold exists to prevent.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (mut adapter, state) = flying_adapter(&fc);
    let mut buf = [0u8; 128];
    arm_and_ramp_east(&mut adapter, &fc, &mut buf);

    for bad in [
        [f32::NAN, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0],
        [0.0, 0.0, f32::NEG_INFINITY],
    ] {
        set_velocity(&state, bad);
        adapter.apply_control(&centered_frame());
        expect_brake_frame(&fc, &mut buf, "non-finite velocity frame");
    }
}

#[test]
fn hold_captures_only_at_or_below_the_speed_threshold() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let (mut adapter, state) = flying_adapter(&fc);
    let mut buf = [0u8; 128];
    arm_and_ramp_east(&mut adapter, &fc, &mut buf);

    // Just above the 0.3 m/s capture threshold: still braking.
    set_velocity(&state, [0.301, 0.0, 0.0]);
    adapter.apply_control(&centered_frame());
    expect_brake_frame(&fc, &mut buf, "above-threshold frame");

    // Just below: stillness demonstrated, hold captured.
    set_velocity(&state, [0.299, 0.0, 0.0]);
    adapter.apply_control(&centered_frame());
    fc.recv_from(&mut buf).expect("hold frame");
    assert_eq!(
        type_mask(&buf),
        2552,
        "below-threshold speed captures the hold"
    );
}
