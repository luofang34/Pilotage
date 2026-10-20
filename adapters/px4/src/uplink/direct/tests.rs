#![allow(clippy::expect_used, clippy::panic)]

//! Direct flight on the multicopter stream against a fake PX4 socket: the
//! thrust takeover, the keepalive kind, clamping, and the hand back to the
//! velocity stream.

use std::net::UdpSocket;
use std::time::Duration;

use super::{DirectDemand, DirectDisposition, MAX_BODY_RATE_RPS, euler_of};
use crate::uplink::Px4Uplink;

const SET_ATTITUDE_TARGET: u32 = 82;
const SET_POSITION_TARGET: u32 = 84;

fn armed_uplink() -> (Px4Uplink, UdpSocket) {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut uplink = Px4Uplink::new(
        crate::config::Px4Profile::Simulation,
        fc.local_addr().expect("addr"),
    )
    .expect("uplink");
    uplink.use_manual_clock();
    uplink.begin_arm(0.0);
    (uplink, fc)
}

/// The next setpoint frame's message id and thrust (SET_ATTITUDE_TARGET
/// only), skipping heartbeats and commands.
fn next_setpoint(fc: &UdpSocket) -> (u32, Option<f32>) {
    let mut buf = [0u8; 128];
    loop {
        let (len, _) = fc.recv_from(&mut buf).expect("a setpoint");
        assert!(len > 10, "runt frame");
        let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
        match msg_id {
            SET_ATTITUDE_TARGET => {
                let thrust = f32::from_le_bytes(buf[42..46].try_into().expect("four bytes"));
                return (msg_id, Some(thrust));
            }
            SET_POSITION_TARGET => return (msg_id, None),
            _ => {}
        }
    }
}

/// Drains the zero-velocity warmup setpoint begin_arm sent.
fn skip_warmup(fc: &UdpSocket) {
    assert_eq!(next_setpoint(fc).0, SET_POSITION_TARGET);
}

const LEVEL: DirectDemand = DirectDemand::BodyRate([0.0; 3]);

#[test]
fn direct_flight_needs_an_active_stream_and_a_reported_thrust() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let mut idle = Px4Uplink::new(
        crate::config::Px4Profile::Simulation,
        fc.local_addr().expect("addr"),
    )
    .expect("uplink");
    idle.direct.observe_thrust(0.6);
    assert_eq!(idle.send_direct(LEVEL, 0.6), DirectDisposition::UplinkIdle);

    let (mut uplink, _fc) = armed_uplink();
    assert_eq!(
        uplink.send_direct(LEVEL, 0.6),
        DirectDisposition::ThrustUnknown,
        "no thrust report yet: nothing to start from"
    );
}

#[test]
fn the_first_direct_frame_holds_the_reported_thrust_until_the_operator_crosses_it() {
    let (mut uplink, fc) = armed_uplink();
    skip_warmup(&fc);
    uplink.direct.observe_thrust(0.62);
    // An operator thrust far below hover is not flown: the hover thrust is.
    assert_eq!(
        uplink.send_direct(LEVEL, 0.1),
        DirectDisposition::Constrained
    );
    assert_eq!(next_setpoint(&fc), (SET_ATTITUDE_TARGET, Some(0.62)));
    assert_eq!(
        uplink.send_direct(LEVEL, 0.4),
        DirectDisposition::Constrained
    );
    assert_eq!(uplink.direct_thrust(), Some(0.62));
    // Crossing the held value hands thrust over.
    assert_eq!(uplink.send_direct(LEVEL, 0.7), DirectDisposition::Flown);
    assert_eq!(next_setpoint(&fc).1, Some(0.62));
    assert_eq!(next_setpoint(&fc), (SET_ATTITUDE_TARGET, Some(0.7)));
    assert_eq!(uplink.send_direct(LEVEL, 0.3), DirectDisposition::Flown);
    assert_eq!(uplink.direct_thrust(), Some(0.3));
}

#[test]
fn an_operator_thrust_at_the_held_value_takes_over_at_once() {
    let (mut uplink, _fc) = armed_uplink();
    uplink.direct.observe_thrust(0.62);
    assert_eq!(uplink.send_direct(LEVEL, 0.61), DirectDisposition::Flown);
    assert_eq!(uplink.direct_thrust(), Some(0.61));
}

#[test]
fn the_keepalive_resends_the_direct_kind_not_a_velocity() {
    let (mut uplink, fc) = armed_uplink();
    skip_warmup(&fc);
    uplink.direct.observe_thrust(0.5);
    uplink.send_direct(DirectDemand::BodyRate([0.2, 0.0, 0.0]), 0.5);
    assert_eq!(next_setpoint(&fc).0, SET_ATTITUDE_TARGET);
    uplink.advance_clock(Duration::from_millis(60));
    uplink.maintain();
    assert_eq!(next_setpoint(&fc).0, SET_ATTITUDE_TARGET);
}

#[test]
fn rates_beyond_the_bound_clamp_and_report_constrained() {
    let (mut uplink, _fc) = armed_uplink();
    uplink.direct.observe_thrust(0.5);
    let wild = DirectDemand::BodyRate([2.0 * MAX_BODY_RATE_RPS, f32::NAN, 0.0]);
    assert_eq!(
        uplink.send_direct(wild, 0.5),
        DirectDisposition::Constrained
    );
    let Some((DirectDemand::BodyRate(rates), _, _)) = uplink.direct.active else {
        panic!("a body-rate demand is active");
    };
    assert_eq!(rates, [MAX_BODY_RATE_RPS, 0.0, 0.0]);
}

#[test]
fn leaving_direct_flight_seeds_the_velocity_stream_from_the_measured_state() {
    let (mut uplink, fc) = armed_uplink();
    skip_warmup(&fc);
    uplink.direct.observe_thrust(0.5);
    uplink.send_direct(LEVEL, 0.5);
    next_setpoint(&fc);
    uplink.enter_velocity(1.2, Some([0.8, -0.3, 0.1]));
    assert!(uplink.direct.active.is_none());
    assert_eq!(uplink.heading_sp_rad, 1.2);
    assert_eq!(uplink.last_vel_ned, [0.8, -0.3, 0.1]);
    uplink.advance_clock(Duration::from_millis(60));
    uplink.maintain();
    assert_eq!(next_setpoint(&fc).0, SET_POSITION_TARGET);
}

#[test]
fn a_new_direct_phase_takes_over_thrust_afresh() {
    let (mut uplink, _fc) = armed_uplink();
    uplink.direct.observe_thrust(0.5);
    uplink.send_direct(LEVEL, 0.5);
    uplink.enter_velocity(0.0, None);
    uplink.direct.observe_thrust(0.66);
    assert_eq!(
        uplink.send_direct(LEVEL, 0.5),
        DirectDisposition::Constrained
    );
    assert_eq!(uplink.direct_thrust(), Some(0.66));
}

#[test]
fn euler_angles_round_trip_a_known_attitude() {
    // 10° roll, -20° pitch, 30° yaw (ZYX).
    let (roll, pitch, yaw) = (
        10f32.to_radians(),
        (-20f32).to_radians(),
        30f32.to_radians(),
    );
    let (sr, cr) = (roll * 0.5).sin_cos();
    let (sp, cp) = (pitch * 0.5).sin_cos();
    let (sy, cy) = (yaw * 0.5).sin_cos();
    let q = [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ];
    let [r, p, y] = euler_of(q);
    for (got, want) in [(r, roll), (p, pitch), (y, yaw)] {
        assert!((got - want).abs() < 1e-5, "{got} vs {want}");
    }
}
