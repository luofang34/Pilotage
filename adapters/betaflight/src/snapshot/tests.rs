#![allow(clippy::expect_used, clippy::panic)]

use std::time::Instant;

use pilotage_msp::message::permanent_id;
use pilotage_msp::{ArmingBlocker, Attitude, BoxIds};

use super::{FcSnapshot, PollReplies, frd_attitude, frd_rates};

/// A status payload with `modes` (box-order bits) and arming flags.
fn status(modes: u32, arming_disable: u32) -> Vec<u8> {
    let mut payload = vec![0x8c, 0x03, 0, 0, 0x2f, 0];
    payload.extend(modes.to_le_bytes());
    payload.extend([0, 0, 0, 4, 0, 0, 30]);
    payload.extend(arming_disable.to_le_bytes());
    payload.push(0);
    payload
}

fn i16s(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn gps(satellites: u8, speed_cm_s: u16, course_decideg: u16) -> Vec<u8> {
    let mut payload = vec![1, satellites];
    payload.extend(473_977_420i32.to_le_bytes());
    payload.extend(85_459_914i32.to_le_bytes());
    payload.extend(488u16.to_le_bytes());
    payload.extend(speed_cm_s.to_le_bytes());
    payload.extend(course_decideg.to_le_bytes());
    payload
}

fn altitude(cm: i32, vario_cm_s: i16) -> Vec<u8> {
    let mut payload = cm.to_le_bytes().to_vec();
    payload.extend(vario_cm_s.to_le_bytes());
    payload
}

/// The SITL's box order: ARM, ANGLE, HORIZON, MAG, ALTHOLD, ...
fn boxes() -> BoxIds {
    BoxIds(vec![0, 1, 2, 5, 3, 6, 27, 11])
}

#[test]
fn a_poll_decodes_into_frd_ned_and_si_units() {
    let status = status(0b11, ArmingBlocker::Angle.bit());
    let attitude = i16s(&[100, -150, 90]);
    let imu = i16s(&[0, 0, 256, 10, 20, 30, 0, 0, 0]);
    let altitude = altitude(1_234, -56);
    let gps = gps(12, 200, 900);
    let snapshot = FcSnapshot::decode(
        PollReplies {
            status: &status,
            attitude: &attitude,
            imu: &imu,
            altitude: &altitude,
            gps: &gps,
        },
        &boxes(),
        Instant::now(),
        7,
        2,
    )
    .expect("decodes");
    assert!(snapshot.armed);
    assert_eq!(snapshot.cycle_time_us, 908);
    assert!(snapshot.arming_disable.contains(ArmingBlocker::Angle));
    assert!((snapshot.roll.to_degrees() - 10.0).abs() < 1e-9);
    assert!((snapshot.pitch.to_degrees() - 15.0).abs() < 1e-9, "nose up");
    assert!((snapshot.heading.to_degrees() - 90.0).abs() < 1e-9);
    assert!((snapshot.altitude_m - 12.34).abs() < 1e-9);
    assert!((snapshot.climb_rate + 0.56).abs() < 1e-9);
    let fix = snapshot.gnss.expect("12 satellites is a fix");
    assert!(fix.velocity_ne[0].abs() < 1e-9 && (fix.velocity_ne[1] - 2.0).abs() < 1e-9);
    assert_eq!((snapshot.sequence, snapshot.epoch), (7, 2));
}

#[test]
fn a_disarmed_status_and_a_weak_fix_read_as_such() {
    let status = status(0b10, 0);
    let attitude = i16s(&[0, 0, 0]);
    let imu = i16s(&[0; 9]);
    let altitude = altitude(0, 0);
    let gps = gps(4, 0, 0);
    let snapshot = FcSnapshot::decode(
        PollReplies {
            status: &status,
            attitude: &attitude,
            imu: &imu,
            altitude: &altitude,
            gps: &gps,
        },
        &boxes(),
        Instant::now(),
        0,
        1,
    )
    .expect("decodes");
    assert!(!snapshot.armed, "only ANGLE is active");
    assert!(snapshot.gnss.is_none(), "four satellites are not a fix");
    assert!(!BoxIds(vec![1]).is_active(pilotage_msp::ModeFlags(1), permanent_id::ARM));
}

#[test]
fn betaflight_pitch_and_gyro_axes_turn_into_frd() {
    let (roll, pitch, heading) = frd_attitude(Attitude {
        roll_decidegrees: -200,
        pitch_decidegrees: 300,
        heading_degrees: 359,
    });
    assert!((roll.to_degrees() + 20.0).abs() < 1e-9);
    assert!((pitch.to_degrees() + 30.0).abs() < 1e-9, "30 deg nose down");
    assert!((heading.to_degrees() - 359.0).abs() < 1e-9);
    let rates = frd_rates([10, 20, 30]).map(f64::to_degrees);
    assert!((rates[0] - 10.0).abs() < 1e-9);
    assert!(
        (rates[1] + 20.0).abs() < 1e-9,
        "Betaflight pitch is nose down"
    );
    assert!(
        (rates[2] + 30.0).abs() < 1e-9,
        "Betaflight yaw is counter-clockwise"
    );
}

#[test]
fn the_quaternion_points_the_nose_along_the_heading() {
    let mut snapshot = sample_snapshot();
    snapshot.heading = 90f64.to_radians();
    let [w, x, y, z] = snapshot.quaternion();
    // The body x axis rotated into NED: first column of the rotation.
    let nose = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
    ];
    assert!(nose[0].abs() < 1e-9 && (nose[1] - 1.0).abs() < 1e-9 && nose[2].abs() < 1e-9);
    snapshot.heading = 0.0;
    snapshot.pitch = 30f64.to_radians();
    let [w, x, y, z] = snapshot.quaternion();
    let nose_down = 2.0 * (x * z - w * y);
    assert!(
        (nose_down + 0.5).abs() < 1e-9,
        "nose up points the nose at -down"
    );
    let norm = w * w + x * x + y * y + z * z;
    assert!((norm - 1.0).abs() < 1e-12);
}

/// A level, armed, fixed snapshot for tests elsewhere in the crate.
pub(crate) fn sample_snapshot() -> FcSnapshot {
    FcSnapshot {
        received_at: Instant::now(),
        sequence: 1,
        epoch: 1,
        cycle_time_us: 908,
        armed: true,
        arming_disable: pilotage_msp::ArmingDisableFlags(0),
        roll: 0.0,
        pitch: 0.0,
        heading: 0.0,
        rates_frd: [0.0; 3],
        altitude_m: 3.0,
        climb_rate: 0.5,
        gnss: Some(super::GnssFix {
            lat_deg: 47.397_742,
            lon_deg: 8.545_594,
            satellites: 12,
            velocity_ne: [1.0, 2.0],
        }),
    }
}
