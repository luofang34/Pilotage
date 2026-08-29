#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::Digest;

const CONDITION: Digest = Digest::from_bytes([7; 32]);

fn accelerometer(axis: SensorAxis, amplitude: f64, interval: u32) -> SensorNoiseLane {
    SensorNoiseLane::Accelerometer {
        axis,
        peak_amplitude_mps2: amplitude,
        update_interval_samples: interval,
    }
}

#[test]
fn an_empty_or_duplicate_lane_list_is_refused() {
    assert!(matches!(
        SensorCondition::BoundedNoise { lanes: Vec::new() }.validate(),
        Err(ValidationError::EmptyList { .. })
    ));

    let lane = accelerometer(SensorAxis::X, 0.1, 10);
    assert!(matches!(
        SensorCondition::BoundedNoise {
            lanes: vec![lane, lane],
        }
        .validate(),
        Err(ValidationError::DuplicateItem { .. })
    ));

    // A duplicate is the same lane, not the same request: a second entry for
    // one lane with a different amplitude has no single applied value.
    assert!(matches!(
        SensorCondition::BoundedNoise {
            lanes: vec![lane, accelerometer(SensorAxis::X, 0.2, 20)],
        }
        .validate(),
        Err(ValidationError::DuplicateItem { .. })
    ));

    SensorCondition::BoundedNoise {
        lanes: vec![lane, accelerometer(SensorAxis::Y, 0.1, 10)],
    }
    .validate()
    .expect("separate axes");
}

#[test]
fn every_lane_has_a_finite_positive_bounded_amplitude() {
    for lane in [
        accelerometer(SensorAxis::X, 20.1, 1),
        SensorNoiseLane::Gyroscope {
            axis: SensorAxis::Y,
            peak_amplitude_rad_s: 10.1,
            update_interval_samples: 1,
        },
        SensorNoiseLane::Magnetometer {
            axis: SensorAxis::Z,
            peak_amplitude_gauss: 2.1,
            update_interval_samples: 1,
        },
        SensorNoiseLane::AbsolutePressure {
            peak_amplitude_hpa: 200.1,
            update_interval_samples: 1,
        },
        SensorNoiseLane::DifferentialPressure {
            peak_amplitude_hpa: 200.1,
            update_interval_samples: 1,
        },
        SensorNoiseLane::PressureAltitude {
            peak_amplitude_m: 2_000.1,
            update_interval_samples: 1,
        },
    ] {
        assert!(matches!(
            SensorCondition::BoundedNoise { lanes: vec![lane] }.validate(),
            Err(ValidationError::OutOfRange { .. })
        ));
    }

    for (amplitude, interval) in [
        (0.0, 1),
        (-0.1, 1),
        (f64::NAN, 1),
        (f64::INFINITY, 1),
        (0.1, 0),
        (0.1, 100_001),
    ] {
        assert!(
            SensorCondition::BoundedNoise {
                lanes: vec![accelerometer(SensorAxis::X, amplitude, interval)],
            }
            .validate()
            .is_err()
        );
    }

    SensorCondition::BoundedNoise {
        lanes: vec![accelerometer(SensorAxis::X, f64::from_bits(1), 1)],
    }
    .validate()
    .expect("the smallest positive amplitude is a request");
}

#[test]
fn a_scalar_lane_refuses_an_axis_and_a_vector_lane_needs_one() {
    let scalar = serde_json::json!({
        "kind": "bounded_noise",
        "lanes": [{
            "sensor": "absolute_pressure",
            "axis": "x",
            "peak_amplitude_hpa": 1.0,
            "update_interval_samples": 10
        }]
    });
    assert!(serde_json::from_value::<SensorCondition>(scalar).is_err());

    let vector = serde_json::json!({
        "kind": "bounded_noise",
        "lanes": [{
            "sensor": "gyroscope",
            "peak_amplitude_rad_s": 0.1,
            "update_interval_samples": 10
        }]
    });
    assert!(serde_json::from_value::<SensorCondition>(vector).is_err());
}

#[test]
fn every_lane_tag_is_separate_and_a_scalar_tag_has_no_axis() {
    let lanes = [
        SensorReferenceLane::Accelerometer(SensorAxis::X),
        SensorReferenceLane::Accelerometer(SensorAxis::Y),
        SensorReferenceLane::Accelerometer(SensorAxis::Z),
        SensorReferenceLane::Gyroscope(SensorAxis::X),
        SensorReferenceLane::Gyroscope(SensorAxis::Y),
        SensorReferenceLane::Gyroscope(SensorAxis::Z),
        SensorReferenceLane::Magnetometer(SensorAxis::X),
        SensorReferenceLane::Magnetometer(SensorAxis::Y),
        SensorReferenceLane::Magnetometer(SensorAxis::Z),
        SensorReferenceLane::AbsolutePressure,
        SensorReferenceLane::DifferentialPressure,
        SensorReferenceLane::PressureAltitude,
    ];
    let tags = lanes
        .iter()
        .map(|lane| lane.tag())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(tags.len(), lanes.len());
    assert_eq!(SensorReferenceLane::AbsolutePressure.tag()[1], 0);
    assert_eq!(SensorReferenceLane::DifferentialPressure.tag()[1], 0);
    assert_eq!(SensorReferenceLane::PressureAltitude.tag()[1], 0);
}

#[test]
fn a_lane_holds_its_offset_for_a_complete_update_interval() {
    let lane = accelerometer(SensorAxis::Z, 0.5, 4);
    let offsets = (0..12)
        .map(|sample| SensorNoiseReference::new(CONDITION, 3, sample, lane))
        .collect::<Vec<_>>();

    for window in offsets.chunks(4) {
        for reference in window {
            assert_eq!(reference.update_index(), window[0].update_index());
            assert!((reference.offset() - window[0].offset()).abs() < f64::EPSILON);
        }
    }
    assert_ne!(offsets[0].update_index(), offsets[4].update_index());
    assert!((offsets[0].offset() - offsets[4].offset()).abs() > f64::EPSILON);
    for reference in &offsets {
        assert_eq!(
            reference.lane(),
            SensorReferenceLane::Accelerometer(SensorAxis::Z)
        );
        assert!(reference.offset().abs() <= 0.5);
    }
}

#[test]
fn the_same_seed_produces_the_same_sensor_offsets() {
    let lane = accelerometer(SensorAxis::X, 0.25, 2);
    let first = SensorNoiseReference::new(CONDITION, 11, 9, lane);
    let repeated = SensorNoiseReference::new(CONDITION, 11, 9, lane);
    let other_run = SensorNoiseReference::new(CONDITION, 12, 9, lane);
    let other_condition = SensorNoiseReference::new(Digest::from_bytes([8; 32]), 11, 9, lane);
    let other_lane =
        SensorNoiseReference::new(CONDITION, 11, 9, accelerometer(SensorAxis::Y, 0.25, 2));

    assert_eq!(first, repeated);
    assert_ne!(first.offset(), other_run.offset());
    assert_ne!(first.offset(), other_condition.offset());
    assert_ne!(first.offset(), other_lane.offset());
}
