//! Link-loss enactment truth and hold-state invalidation at the adapter
//! boundary: a refused neutral send is a typed failure (never silent
//! success), and any link-loss transition invalidates a captured
//! position-hold point — a hold surviving the loss would command
//! recovery back toward an obsolete point the instant control resumes.

use std::time::Duration;

use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, VehicleId};

use super::super::AviateAdapter;
use super::fixtures::{flight_frame, state_with};

#[test]
fn link_loss_enactment_reports_refused_sends_and_proves_sent_ones() {
    // Port zero is unroutable for send_to: the neutral never leaves, so
    // enactment must be a typed failure while the latch still engages.
    let mut refused = crate::uplink::FlightUplink::new().expect("uplink");
    refused.set_target("0.0.0.0:0".parse().expect("addr"));
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(refused);
    let result = adapter.set_link_loss_policy(
        VehicleId::new(1),
        Some(pilotage_adapter_api::LinkLossPolicy::Neutralize),
    );
    assert!(
        matches!(
            result,
            Err(pilotage_adapter_api::LinkLossEnactError::ChannelRejected { .. })
        ),
        "refused send must be a typed failure, got {result:?}"
    );
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "the latch engages even when the send was refused"
    );

    // Accepted send: a reachable fake FC receives the zero-velocity
    // setpoint and the enactment reports Ok.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut sent = crate::uplink::FlightUplink::new().expect("uplink");
    sent.set_target(fc.local_addr().expect("addr"));
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(sent);
    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            Some(pilotage_adapter_api::LinkLossPolicy::Neutralize),
        )
        .expect("neutral datagram accepted by the socket");
    let mut buf = [0u8; 128];
    fc.recv_from(&mut buf).expect("neutral frame");
    let mask = u16::from_le_bytes([buf[10 + 48], buf[11 + 48]]);
    assert_eq!(mask, 2503, "neutralize streams velocity mode");
}

#[test]
fn link_loss_transitions_invalidate_the_captured_hold_point() {
    // Contract level: seeding a hold and driving either transition must
    // leave the uplink hold-free.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    uplink.seed_hold_for_test([10.0, 20.0, -30.0]);
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(uplink);

    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            Some(pilotage_adapter_api::LinkLossPolicy::Neutralize),
        )
        .expect("engage");
    assert!(
        !adapter.uplink_hold_captured(),
        "engaging link loss must invalidate the captured hold"
    );

    adapter
        .set_link_loss_policy(VehicleId::new(1), None)
        .expect("clear");
    assert!(
        !adapter.uplink_hold_captured(),
        "clearing link loss must also start hold-free"
    );
}

/// Stamps the fixture as (near-)still at `pos`, fresh as of now.
fn set_still_at(
    state: &std::sync::Arc<std::sync::Mutex<pilotage_mavlink::link::LinkState>>,
    pos: [f32; 3],
) {
    let mut latest = state.lock().expect("state lock");
    let kin = latest.kinematics.as_mut().expect("kinematics fixture");
    kin.pos_ned_m = pos;
    kin.vel_ned_mps = [0.1, 0.0, 0.0];
    kin.received_at = std::time::Instant::now();
}

/// Sends one centered frame, expects a position-hold setpoint back, and
/// returns its north coordinate.
fn centered_hold_north(
    adapter: &mut AviateAdapter,
    fc: &std::net::UdpSocket,
    buf: &mut [u8; 128],
) -> f32 {
    adapter.apply_control(&flight_frame(
        vec![(LogicalAxisId::new(super::super::PITCH_AXIS), 0.0)],
        vec![],
    ));
    fc.recv_from(buf).expect("hold frame");
    let mask = u16::from_le_bytes([buf[10 + 48], buf[11 + 48]]);
    assert_eq!(mask, 2552, "still vehicle captures a hold");
    f32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]])
}

#[test]
fn a_hold_captured_before_link_loss_never_commands_recovery_to_it() {
    // Behavior level: capture a hold in flight, lose and recover the
    // link while the vehicle "drifts" (the fixture position moves), and
    // prove the first post-recovery hold is at the NEW position — a
    // stale hold would fly the vehicle back to the obsolete point.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("addr"));
    let state = state_with(Duration::ZERO, Duration::ZERO);
    let mut adapter =
        AviateAdapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink);

    // Arm, expire the post-arm quiet window deterministically (no
    // wall-clock sleep), then open the motors-idle gate with a climb
    // frame.
    adapter.apply_control(&flight_frame(
        vec![],
        vec![(
            LogicalButtonId::new(super::super::ARM_BUTTON),
            ButtonEdge::Pressed,
        )],
    ));
    let mut buf = [0u8; 128];
    fc.recv_from(&mut buf).expect("arm frame");
    adapter.expire_uplink_quiet_for_test();
    adapter.apply_control(&flight_frame(
        vec![(LogicalAxisId::new(super::super::THROTTLE_AXIS), 0.5)],
        vec![],
    ));
    fc.recv_from(&mut buf).expect("climb frame");

    // Still (fixture velocity low) with sticks centered: hold captures
    // at the fixture position (10, 20, -30).
    {
        let mut latest = state.lock().expect("state lock");
        let kin = latest.kinematics.as_mut().expect("kinematics fixture");
        kin.vel_ned_mps = [0.1, 0.0, 0.0];
        kin.received_at = std::time::Instant::now();
    }
    adapter.apply_control(&flight_frame(
        vec![(LogicalAxisId::new(super::super::PITCH_AXIS), 0.0)],
        vec![],
    ));
    fc.recv_from(&mut buf).expect("hold frame");
    let mask = u16::from_le_bytes([buf[10 + 48], buf[11 + 48]]);
    assert_eq!(mask, 2552, "still vehicle captures a hold");
    let north = f32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]);
    assert!(
        (north - 10.0).abs() < 1e-3,
        "hold at the old point: {north}"
    );

    // Link loss engages (drain its neutral), the vehicle drifts while
    // neutralized, and the host clears after the new holder's activation.
    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            Some(pilotage_adapter_api::LinkLossPolicy::Neutralize),
        )
        .expect("engage");
    fc.recv_from(&mut buf).expect("link-loss neutral frame");
    set_still_at(&state, [50.0, 60.0, -30.0]);
    adapter
        .set_link_loss_policy(VehicleId::new(1), None)
        .expect("clear");

    // First centered frame after recovery: the hold captures at the
    // CURRENT position, never the pre-loss one.
    let north = centered_hold_north(&mut adapter, &fc, &mut buf);
    assert!(
        (north - 50.0).abs() < 1e-3,
        "hold north must be the drifted position, got {north}"
    );
}
