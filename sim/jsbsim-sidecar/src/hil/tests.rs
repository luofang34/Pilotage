//! The HIL messages byte for byte where PX4 reads them, and the stream
//! reader over split, foreign, and corrupt frames.

#![allow(clippy::unwrap_used)]

use pilotage_mavlink::codec::{compute_crc, encode_frame_v2_from};

use super::{
    ActuatorControls, Encoder, FIELDS_IMU, FrameReader, HIL_ACTUATOR_CONTROLS, HIL_GPS, HIL_SENSOR,
    HIL_STATE_QUATERNION,
};
use crate::rotation::Euler;
use crate::sensors::{GnssSample, InertialSample};
use crate::truth::Truth;

fn f32_at(frame: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(frame[10 + at..14 + at].try_into().unwrap())
}

fn i32_at(frame: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(frame[10 + at..14 + at].try_into().unwrap())
}

fn check_frame(frame: &[u8], (id, extra): (u32, u8), payload_len: usize) {
    assert_eq!(frame[0], 0xFD);
    assert_eq!(usize::from(frame[1]), payload_len);
    let got = u32::from(frame[7]) | u32::from(frame[8]) << 8 | u32::from(frame[9]) << 16;
    assert_eq!(got, id);
    let crc = compute_crc(&frame[1..10 + payload_len], extra);
    assert_eq!(&frame[10 + payload_len..], &crc.to_le_bytes());
}

#[test]
fn hil_sensor_carries_px4s_fields_in_wire_order() {
    let sample = InertialSample {
        time_us: 123_456,
        accel_frd: [0.1, 0.2, -9.8],
        gyro_frd: [0.01, 0.02, 0.03],
        mag_frd_gauss: [0.2, -0.04, 0.45],
        pressure_hpa: 1006.5,
        pressure_altitude_m: 56.0,
        diff_pressure_hpa: 0.0,
        temperature_c: 14.6,
    };
    let frame = Encoder::default().hil_sensor(&sample, FIELDS_IMU);
    check_frame(&frame, HIL_SENSOR, 65);
    assert_eq!(
        u64::from_le_bytes(frame[10..18].try_into().unwrap()),
        123_456
    );
    assert_eq!(f32_at(&frame, 16), -9.8);
    assert_eq!(f32_at(&frame, 32), 0.2);
    assert_eq!(f32_at(&frame, 44), 1006.5);
    assert_eq!(
        u32::from_le_bytes(frame[10 + 60..10 + 64].try_into().unwrap()),
        FIELDS_IMU
    );
}

#[test]
fn hil_gps_carries_a_3d_fix() {
    let fix = GnssSample {
        time_us: 1,
        latitude_deg: 40.577_186_1,
        longitude_deg: -74.738_191_8,
        altitude_m: 56.3,
        velocity_ned: [3.0, 4.0, -1.0],
        horizontal_accuracy_m: 0.45,
        vertical_accuracy_m: 0.75,
    };
    let frame = Encoder::default().hil_gps(&fix);
    check_frame(&frame, HIL_GPS, 39);
    assert_eq!(i32_at(&frame, 8), 405_771_861);
    assert_eq!(i32_at(&frame, 12), -747_381_918);
    assert_eq!(i32_at(&frame, 16), 56_300);
    // Ground speed 5 m/s, course 053.13°, fix 3D.
    assert_eq!(u16::from_le_bytes([frame[10 + 24], frame[10 + 25]]), 500);
    assert_eq!(u16::from_le_bytes([frame[10 + 32], frame[10 + 33]]), 5313);
    assert_eq!(frame[10 + 34], 3);
}

#[test]
fn hil_state_quaternion_carries_the_truth_attitude() {
    let truth = Truth {
        time_s: 2.0,
        latitude_deg: 40.5,
        longitude_deg: -74.7,
        altitude_m: 60.0,
        velocity_ned: [1.0, 0.0, 0.0],
        attitude: Euler {
            roll: 0.0,
            pitch: 0.0,
            yaw: std::f64::consts::FRAC_PI_2,
        },
        rates_frd: [0.0; 3],
        specific_force_frd: [0.0, 0.0, -9.806_65],
        pressure_pa: 101_325.0,
        temperature_c: 15.0,
        dynamic_pressure_pa: 0.0,
        true_airspeed_mps: 0.0,
        calibrated_airspeed_mps: 0.0,
    };
    let frame = Encoder::default().hil_state_quaternion(&truth);
    check_frame(&frame, HIL_STATE_QUATERNION, 64);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!((f32_at(&frame, 8) - half).abs() < 1e-6);
    assert!((f32_at(&frame, 20) - half).abs() < 1e-6);
    assert_eq!(i32_at(&frame, 44), 60_000);
    // 1 g up on the accelerometer z, in milli-g.
    assert_eq!(i16::from_le_bytes([frame[10 + 62], frame[10 + 63]]), -1000);
}

fn actuator_frame(seq: u8, first: f32, mode: u8) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&77u64.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    for i in 0..16 {
        payload.extend_from_slice(&(if i == 0 { first } else { 0.0f32 }).to_le_bytes());
    }
    payload.push(mode);
    let mut out = vec![0u8; 12 + payload.len()];
    let n = encode_frame_v2_from(
        seq,
        HIL_ACTUATOR_CONTROLS.0,
        &payload,
        HIL_ACTUATOR_CONTROLS.1,
        1,
        1,
        &mut out,
    );
    out.truncate(n);
    out
}

#[test]
fn the_reader_yields_actuator_controls_across_split_and_foreign_frames() {
    let mut reader = FrameReader::default();
    let mut stream = vec![0x00, 0x13];
    stream.extend(Encoder::default().heartbeat());
    stream.extend(actuator_frame(0, 0.75, 0x80));
    let (head, tail) = stream.split_at(20);
    reader.push(head);
    assert!(reader.next_actuator_controls().is_none());
    reader.push(tail);
    let controls = reader.next_actuator_controls().unwrap();
    assert_eq!(controls.controls[0], 0.75);
    assert!(controls.armed());
    assert_eq!(controls.flags, 1);
    assert_eq!(controls.time_us, 77);
    assert!(reader.next_actuator_controls().is_none());
}

#[test]
fn a_corrupt_frame_is_dropped_and_the_next_one_read() {
    let mut reader = FrameReader::default();
    let mut bad = actuator_frame(0, 0.5, 0x80);
    let last = bad.len() - 1;
    bad[last] ^= 0xff;
    reader.push(&bad);
    reader.push(&actuator_frame(1, 0.25, 0));
    let controls = reader.next_actuator_controls().unwrap();
    assert_eq!(controls.controls[0], 0.25);
    assert!(!controls.armed());
    assert_eq!(reader.crc_errors, 1);
}

#[test]
fn a_trimmed_payload_decodes_with_zeros() {
    let controls = ActuatorControls::decode(&[0u8; 20]);
    assert_eq!(controls.mode, 0);
    assert!(controls.controls.iter().all(|&c| c == 0.0));
}
