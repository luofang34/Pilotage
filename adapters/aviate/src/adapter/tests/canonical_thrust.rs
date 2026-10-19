//! Direct-flight thrust is canonical: a centred stick (thrust 0.5) commands
//! each airframe's own hover collective from the first frame, so a switch
//! into direct flight neither steps the thrust nor climbs or sinks.

use std::net::UdpSocket;
use std::time::Duration;

use pilotage_control_feel::{
    AirframeLimits, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile,
};

use crate::SimulatorDirectAuthority;
use crate::adapter::AviateProfile;
use crate::uplink::FlightUplink;

fn fake_fc() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("read timeout");
    socket
}

fn uplink_for(fc: &UdpSocket, airframe: AirframeLimits) -> FlightUplink {
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped_for(
        airframe,
        FeelMode::Balanced,
    ))
    .expect("a valid shaped profile");
    let mut uplink = FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    let authority = SimulatorDirectAuthority::for_profile(AviateProfile::Simulation)
        .expect("simulation authority");
    uplink.open_setpoint_stream(&authority);
    uplink
}

/// The collective of the next SET_ATTITUDE_TARGET the FC receives.
fn next_thrust(fc: &UdpSocket) -> f32 {
    let mut buffer = [0_u8; 128];
    let (len, _) = fc.recv_from(&mut buffer).expect("a frame reached the FC");
    assert!(len > 46 && buffer[7] == 82, "a SET_ATTITUDE_TARGET");
    f32::from_le_bytes([buffer[42], buffer[43], buffer[44], buffer[45]])
}

#[test]
fn centre_thrust_is_each_airframes_hover_from_the_first_frame() {
    for airframe in AirframeLimits::ALL {
        let fc = fake_fc();
        let mut uplink = uplink_for(&fc, airframe);
        uplink.send_attitude_frame(0.0, 0.0, 0.0, 0.5);
        let entry = next_thrust(&fc);
        assert!(
            (entry - airframe.hover_thrust).abs() < 1e-6,
            "{}: entry collective {entry}, hover {}",
            airframe.id,
            airframe.hover_thrust
        );
        for _ in 0..50 {
            uplink.advance_clock(Duration::from_millis(20));
            uplink.send_attitude_frame(0.0, 0.0, 0.0, 0.5);
            let held = next_thrust(&fc);
            assert!(
                (held - airframe.hover_thrust).abs() < 1e-6,
                "{}: centred collective drifted to {held}",
                airframe.id
            );
        }
    }
}

#[test]
fn thrust_above_and_below_centre_brackets_the_x500_hover() {
    let fc = fake_fc();
    let mut uplink = uplink_for(&fc, AirframeLimits::X500);
    uplink.send_attitude_frame(0.0, 0.0, 0.0, 0.5);
    next_thrust(&fc);
    let mut last = 0.0;
    for (thrust, below) in [(0.3, true), (0.7, false)] {
        for _ in 0..100 {
            uplink.advance_clock(Duration::from_millis(20));
            uplink.send_attitude_frame(0.0, 0.0, 0.0, thrust);
            last = next_thrust(&fc);
        }
        let hover = AirframeLimits::X500.hover_thrust;
        assert_eq!(
            last < hover,
            below,
            "thrust {thrust} gave collective {last}"
        );
    }
}

#[test]
fn a_profile_hovering_at_no_known_airframes_trim_is_refused() {
    let mut profile = FlightFeelProfile::shaped_for(AirframeLimits::X500, FeelMode::Balanced);
    profile.envelope.direct_hover_thrust = 0.65;
    let profile = ValidatedFlightFeelProfile::new(profile).expect("structurally valid");
    assert!(crate::adapter::validate_aviate_profile_bindings(&profile).is_err());
    let x500 = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped_for(
        AirframeLimits::X500,
        FeelMode::Balanced,
    ))
    .expect("valid");
    assert!(crate::adapter::validate_aviate_profile_bindings(&x500).is_ok());
}
