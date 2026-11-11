use navigate_contract::MonotonicNanos;
use pilotage_protocol::wire;

use super::vehicle_status;

const NOW: MonotonicNanos = MonotonicNanos::from_nanos(5_000_000_000);

/// A sample from a flight controller that states its readiness, with an
/// estimate 120 m above the origin flying at 28 m/s under `valid_flags`.
fn sample(ready: Option<bool>, valid_flags: u32) -> wire::TelemetrySample {
    wire::TelemetrySample {
        fc_state: Some(Box::new(wire::FcState {
            ready_to_arm: ready,
            ..wire::FcState::default()
        })),
        avionics: Some(wire::AvionicsState {
            pos_d_m: -120.0,
            indicated_airspeed_mps: 28.0,
            valid_flags,
            kinematics_stamp: Some(wire::MeasurementStamp::default()),
            airspeed_stamp: Some(wire::MeasurementStamp::default()),
            estimator_status_stamp: Some(wire::MeasurementStamp::default()),
            ..wire::AvionicsState::default()
        }),
        ..wire::TelemetrySample::default()
    }
}

#[test]
fn a_sample_without_a_stated_readiness_hands_nothing() {
    assert_eq!(vehicle_status(&sample(None, 0x3f), NOW), None);
    let mut no_fc = sample(Some(true), 0x3f);
    no_fc.fc_state = None;
    assert_eq!(vehicle_status(&no_fc, NOW), None);
}

#[test]
fn without_gnss_the_height_and_airspeed_are_read_under_their_own_bits() {
    // Attitude, rates, height, airspeed: no position, no velocity.
    let status = vehicle_status(&sample(Some(true), 0b11_0011), NOW).expect("a status");
    assert!(status.ready_to_arm);
    assert_eq!(status.acquired_at, NOW);
    assert_eq!(status.position_valid, Some(false));
    assert_eq!(status.height_above_origin_m, Some(120.0));
    assert_eq!(status.airspeed_mps, Some(28.0));
}

#[test]
fn a_position_without_the_height_bit_states_no_height() {
    let status = vehicle_status(&sample(Some(false), 0b00_1111), NOW).expect("a status");
    assert!(!status.ready_to_arm);
    assert_eq!(status.position_valid, Some(true));
    assert_eq!(status.height_above_origin_m, None);
    assert_eq!(status.airspeed_mps, None);
}

#[test]
fn a_value_needs_its_group_stamp_and_a_finite_reading() {
    let mut unstamped = sample(Some(true), 0x3f);
    if let Some(avionics) = unstamped.avionics.as_mut() {
        avionics.kinematics_stamp = None;
        avionics.airspeed_stamp = None;
    }
    let status = vehicle_status(&unstamped, NOW).expect("a status");
    assert_eq!(status.height_above_origin_m, None);
    assert_eq!(status.airspeed_mps, None);
    let mut nan = sample(Some(true), 0x3f);
    if let Some(avionics) = nan.avionics.as_mut() {
        avionics.indicated_airspeed_mps = f32::NAN;
        avionics.pos_d_m = f32::NAN;
    }
    let status = vehicle_status(&nan, NOW).expect("a status");
    assert_eq!(status.height_above_origin_m, None);
    assert_eq!(status.airspeed_mps, None);
}

#[test]
fn without_an_estimator_status_the_bits_authorize_nothing() {
    let mut unauthorized = sample(Some(true), 0x3f);
    if let Some(avionics) = unauthorized.avionics.as_mut() {
        avionics.estimator_status_stamp = None;
    }
    let status = vehicle_status(&unauthorized, NOW).expect("a status");
    assert!(status.ready_to_arm);
    assert_eq!(status.position_valid, None);
    assert_eq!(status.height_above_origin_m, None);
    assert_eq!(status.airspeed_mps, None);
}
