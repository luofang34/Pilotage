//! The simulator-only exact direct path, beside the shaped operator paths
//! it must not disturb.
//!
//! Every test here drives ONE uplink. The exact path and the shaped paths
//! share the socket, the frame sequence, and the control-feel envelope, so
//! a comparison between them is a comparison of laws and not of rigs.

use std::net::UdpSocket;
use std::time::Duration;

use pilotage_control_feel::{FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};

use crate::adapter::AviateProfile;
use crate::uplink::FlightUplink;
use crate::{ExactDirectError, ExactDirectSetpoint, SimulatorDirectAuthority};

/// A tilt step large enough that a rate limit cannot deliver it in one
/// frame, and small enough to stay inside the direct tilt envelope.
const STEP_RAD: f32 = 0.3;
/// One simulator sample at the shaped path's frame rate.
const SAMPLE: Duration = Duration::from_millis(20);

fn fake_fc() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("read timeout");
    socket
}

fn uplink_for(fc: &UdpSocket, mode: FeelMode) -> FlightUplink {
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(mode))
        .expect("valid Alia profile");
    let mut uplink = FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    uplink.open_setpoint_stream(&authority());
    uplink
}

fn authority() -> SimulatorDirectAuthority {
    SimulatorDirectAuthority::for_profile(AviateProfile::Simulation).expect("simulation authority")
}

fn field(frame: &[u8], offset: usize) -> f32 {
    let start = 10 + offset;
    let bytes = frame.get(start..start + 4).expect("payload field");
    f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Recovers roll, pitch, yaw and collective from one SET_ATTITUDE_TARGET.
fn attitude_of(frame: &[u8]) -> (f32, f32, f32, f32) {
    assert_eq!(frame[7], 82, "SET_ATTITUDE_TARGET id");
    let (qw, qx, qy, qz) = (
        field(frame, 4),
        field(frame, 8),
        field(frame, 12),
        field(frame, 16),
    );
    let roll = (2.0 * (qw * qx + qy * qz)).atan2(1.0 - 2.0 * (qx * qx + qy * qy));
    let pitch = (2.0 * (qw * qy - qz * qx)).asin();
    let yaw = (2.0 * (qw * qz + qx * qy)).atan2(1.0 - 2.0 * (qy * qy + qz * qz));
    (roll, pitch, yaw, field(frame, 32))
}

fn next_frame(fc: &UdpSocket) -> Vec<u8> {
    let mut buffer = [0_u8; 128];
    let (len, _) = fc.recv_from(&mut buffer).expect("a frame reached the FC");
    buffer[..len].to_vec()
}

fn assert_link_silent(fc: &UdpSocket) {
    let mut buffer = [0_u8; 128];
    fc.set_read_timeout(Some(Duration::from_millis(50)))
        .expect("read timeout");
    let received = fc.recv_from(&mut buffer);
    fc.set_read_timeout(Some(Duration::from_millis(250)))
        .expect("read timeout");
    assert!(
        received.is_err(),
        "a refused exact step must not put a frame on the command link"
    );
}

/// Seeds the shaped direct law at level attitude and drains the frame it
/// sends, so the next shaped frame measures shaping alone.
fn seed_shaped_direct(uplink: &mut FlightUplink, fc: &UdpSocket) {
    uplink.send_attitude_frame(0.0, 0.0, 0.0, 0.72);
    let _seed = next_frame(fc);
}

#[test]
fn the_normal_direct_path_still_applies_its_declared_shaping() {
    let fc = fake_fc();
    let mut uplink = uplink_for(&fc, FeelMode::Balanced);
    seed_shaped_direct(&mut uplink, &fc);

    uplink.advance_clock(SAMPLE);
    uplink.send_attitude_frame(STEP_RAD, 0.0, 0.0, 0.72);
    let (roll, ..) = attitude_of(&next_frame(&fc));

    // The Balanced law bounds the tilt rate at 1.2 rad/s, so one 20 ms
    // sample can deliver at most 0.024 rad of the 0.3 rad request.
    assert!(
        roll < STEP_RAD * 0.2,
        "the shaped path must ramp, not step: {roll} rad"
    );
    assert!(roll > 0.0, "the shaped path must still respond: {roll} rad");
}

#[test]
fn the_simulator_direct_path_sends_an_exact_target_in_one_sample() {
    let fc = fake_fc();
    let mut uplink = uplink_for(&fc, FeelMode::Balanced);
    seed_shaped_direct(&mut uplink, &fc);

    uplink.advance_clock(SAMPLE);
    let transmitted = uplink
        .send_exact_direct_setpoint(
            &authority(),
            ExactDirectSetpoint {
                roll_rad: STEP_RAD,
                pitch_rad: 0.0,
                yaw_rad: 0.0,
                collective_force: 0.72,
            },
        )
        .expect("exact step");
    let (roll, ..) = attitude_of(&next_frame(&fc));

    assert_eq!(
        transmitted.setpoint.roll_rad, STEP_RAD,
        "the transmitted setpoint IS the requested target"
    );
    assert!(
        (roll - STEP_RAD).abs() < 1e-6,
        "the exact path must reach the target in one sample: {roll} rad"
    );
}

#[test]
fn an_exact_step_keeps_every_unrelated_axis_at_its_requested_value() {
    let fc = fake_fc();
    let mut uplink = uplink_for(&fc, FeelMode::Balanced);
    seed_shaped_direct(&mut uplink, &fc);

    uplink.advance_clock(SAMPLE);
    uplink
        .send_exact_direct_setpoint(
            &authority(),
            ExactDirectSetpoint {
                roll_rad: STEP_RAD,
                pitch_rad: -0.1,
                yaw_rad: 0.4,
                collective_force: 0.61,
            },
        )
        .expect("exact step");
    let (roll, pitch, yaw, collective) = attitude_of(&next_frame(&fc));

    assert!((roll - STEP_RAD).abs() < 1e-6, "roll {roll}");
    assert!((pitch + 0.1).abs() < 1e-6, "pitch {pitch}");
    assert!((yaw - 0.4).abs() < 1e-6, "yaw {yaw}");
    assert!((collective - 0.61).abs() < 1e-6, "collective {collective}");
}

#[test]
fn a_changed_default_feel_profile_is_not_necessary_for_a_direct_step() {
    // The compatibility profile is the launch default. An exact step needs
    // no other profile, so a trial never has to change operator feel to
    // measure the direct controller.
    let fc = fake_fc();
    let mut uplink = FlightUplink::new().expect("default uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    uplink.open_setpoint_stream(&authority());

    uplink
        .send_exact_direct_setpoint(
            &authority(),
            ExactDirectSetpoint {
                roll_rad: STEP_RAD,
                pitch_rad: 0.0,
                yaw_rad: 0.0,
                collective_force: 0.72,
            },
        )
        .expect("exact step on the default profile");
    let (roll, ..) = attitude_of(&next_frame(&fc));

    assert!((roll - STEP_RAD).abs() < 1e-6, "roll {roll}");
    assert_eq!(
        uplink.active_profile_for_test().profile().mode,
        FeelMode::LegacyCompatibility,
        "the exact step must not install another profile"
    );
}

#[test]
fn the_simulator_direct_path_leaves_the_shaped_direct_law_untouched() {
    // Two uplinks run the same shaped sequence. One takes an exact step in
    // the middle. If the exact path touched the shaped law's integrators,
    // seed, or heading, the shaped frames would diverge.
    let control_fc = fake_fc();
    let subject_fc = fake_fc();
    let mut control = uplink_for(&control_fc, FeelMode::Balanced);
    let mut subject = uplink_for(&subject_fc, FeelMode::Balanced);
    seed_shaped_direct(&mut control, &control_fc);
    seed_shaped_direct(&mut subject, &subject_fc);

    subject
        .send_exact_direct_setpoint(
            &authority(),
            ExactDirectSetpoint {
                roll_rad: STEP_RAD,
                pitch_rad: 0.0,
                yaw_rad: 0.0,
                collective_force: 0.9,
            },
        )
        .expect("exact step");
    let _exact = next_frame(&subject_fc);

    for _ in 0..3 {
        control.advance_clock(SAMPLE);
        subject.advance_clock(SAMPLE);
        control.send_attitude_frame(STEP_RAD, 0.0, 0.0, 0.72);
        subject.send_attitude_frame(STEP_RAD, 0.0, 0.0, 0.72);
        assert_eq!(
            attitude_of(&next_frame(&control_fc)),
            attitude_of(&next_frame(&subject_fc)),
            "an exact step must not move the shaped direct law"
        );
    }
}

#[test]
fn the_operator_velocity_path_still_uses_normal_shaping() {
    // The same proof for the operator family: an exact direct step must not
    // change the velocity law's response curve, apply dynamics, or
    // integrated heading.
    let control_fc = fake_fc();
    let subject_fc = fake_fc();
    let mut control = uplink_for(&control_fc, FeelMode::Balanced);
    let mut subject = uplink_for(&subject_fc, FeelMode::Balanced);

    let stick = |uplink: &mut FlightUplink| {
        uplink.send_stick_frame(0.0, 1.0, 0.6, 0.5, 0.0, [0.0; 3], Some([0.0; 3]), None);
    };
    stick(&mut control);
    stick(&mut subject);
    let _control_seed = next_frame(&control_fc);
    let _subject_seed = next_frame(&subject_fc);

    subject
        .send_exact_direct_setpoint(
            &authority(),
            ExactDirectSetpoint {
                roll_rad: STEP_RAD,
                pitch_rad: 0.0,
                yaw_rad: 0.0,
                collective_force: 0.9,
            },
        )
        .expect("exact step");
    let _exact = next_frame(&subject_fc);

    let mut commanded = false;
    for _ in 0..3 {
        control.advance_clock(SAMPLE);
        subject.advance_clock(SAMPLE);
        stick(&mut control);
        stick(&mut subject);
        let control_frame = next_frame(&control_fc);
        let subject_frame = next_frame(&subject_fc);
        assert_eq!(
            control_frame[7], subject_frame[7],
            "the operator family must keep its own message"
        );
        // A velocity setpoint carries vx/vy/vz at payload offsets 16..28
        // and the absolute heading at 40.
        for offset in [16, 20, 24, 40] {
            let expected = field(&control_frame, offset);
            commanded = commanded || expected != 0.0;
            assert_eq!(
                expected,
                field(&subject_frame, offset),
                "an exact step must not move the operator velocity law"
            );
        }
    }
    assert!(
        commanded,
        "the operator law must have commanded something for this comparison to mean anything"
    );
}

#[test]
fn a_hardware_target_cannot_mint_exact_direct_authority() {
    assert!(SimulatorDirectAuthority::for_profile(AviateProfile::Physical).is_none());
    assert!(SimulatorDirectAuthority::for_profile(AviateProfile::OracleOnly).is_none());
    assert_eq!(
        authority().profile(),
        AviateProfile::Simulation,
        "only the simulation profile carries exact direct authority"
    );
}

#[test]
fn a_constrained_exact_target_is_refused_before_any_frame_leaves() {
    let fc = fake_fc();
    let mut uplink = uplink_for(&fc, FeelMode::Balanced);
    let level = ExactDirectSetpoint {
        roll_rad: 0.0,
        pitch_rad: 0.0,
        yaw_rad: 0.0,
        collective_force: 0.72,
    };

    let refusals = [
        (
            ExactDirectSetpoint {
                roll_rad: 1.5,
                ..level
            },
            "roll outside the tilt envelope",
        ),
        (
            ExactDirectSetpoint {
                pitch_rad: f32::NAN,
                ..level
            },
            "pitch is not finite",
        ),
        (
            ExactDirectSetpoint {
                yaw_rad: 7.0,
                ..level
            },
            "heading outside one revolution",
        ),
        (
            ExactDirectSetpoint {
                collective_force: 1.4,
                ..level
            },
            "collective outside the normalized range",
        ),
    ];
    for (setpoint, reason) in refusals {
        let result = uplink.send_exact_direct_setpoint(&authority(), setpoint);
        assert!(result.is_err(), "{reason} must be refused");
        assert_link_silent(&fc);
    }
}

#[test]
fn a_closed_setpoint_stream_refuses_an_exact_step() {
    let fc = fake_fc();
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(FeelMode::Balanced))
        .expect("valid Alia profile");
    let mut uplink = FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();

    let result = uplink.send_exact_direct_setpoint(
        &authority(),
        ExactDirectSetpoint {
            roll_rad: 0.0,
            pitch_rad: 0.0,
            yaw_rad: 0.0,
            collective_force: 0.72,
        },
    );

    assert_eq!(result, Err(ExactDirectError::StreamClosed));
    assert_link_silent(&fc);
}
