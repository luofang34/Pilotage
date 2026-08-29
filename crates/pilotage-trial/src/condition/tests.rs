#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::{BackendCapability, HoverEstimatorMode};

#[path = "tests/strict_json.rs"]
mod strict_json;

#[path = "tests/uncertainty.rs"]
mod uncertainty;

fn condition(seed: u64) -> ConditionSet {
    ConditionSet {
        schema_version: CONDITION_SET_SCHEMA_VERSION,
        id: "crosswind-gust".to_owned(),
        revision: 1,
        seed,
        wind: WindCondition {
            steady: HorizontalWind {
                speed_mps: 5.0,
                direction_deg: 270.0,
            },
            gusts: vec![GustEvent {
                start_ns: 1_000_000_000,
                rise_ns: 1_000_000_000,
                hold_ns: 1_000_000_000,
                fall_ns: 1_000_000_000,
                speed_mps: 3.0,
                direction_deg: 270.0,
            }],
            turbulence: TurbulenceModel::BandLimitedNoise {
                amplitude_mps: 0.5,
                knot_interval_ns: 200_000_000,
            },
        },
        timing: TimingCondition::nominal(),
        sensor: SensorCondition::nominal(),
        actuator: ActuatorCondition::nominal(),
        controller_initialization: ControllerInitializationCondition::nominal(),
    }
}

fn wind(value: &ConditionSet, elapsed_ns: u64) -> AppliedWind {
    value.wind_at(elapsed_ns).expect("valid condition")
}

fn run_wind(value: &ConditionSet, run_seed: u64, elapsed_ns: u64) -> AppliedWind {
    value
        .wind_at_for_run(run_seed, elapsed_ns)
        .expect("valid condition")
}

fn source_delay(value: &ConditionSet, run_seed: u64, elapsed_ns: u64) -> u64 {
    value
        .source_delay_ns_for_run(run_seed, elapsed_ns)
        .expect("valid condition")
}

#[test]
fn condition_set_rejects_an_unknown_schema_version() {
    let mut value = condition(42);
    value.schema_version = CONDITION_SET_SCHEMA_VERSION.wrapping_add(1);

    assert!(matches!(
        value.validate(),
        Err(ValidationError::UnsupportedSchemaVersion {
            document: "condition set",
            ..
        })
    ));
}

#[test]
fn the_same_seed_produces_the_same_schedule() {
    let first = condition(42);
    let second = condition(42);
    let first_samples: Vec<_> = (0..100)
        .map(|index| wind(&first, index * 50_000_000))
        .collect();
    let second_samples: Vec<_> = (0..100)
        .map(|index| wind(&second, index * 50_000_000))
        .collect();
    assert_eq!(first_samples, second_samples);
}

#[test]
fn a_different_seed_changes_the_turbulence_schedule() {
    assert_ne!(
        wind(&condition(42), 250_000_000),
        wind(&condition(43), 250_000_000)
    );
}

#[test]
fn a_run_seed_is_repeatable_and_separates_repetitions() {
    let value = condition(42);
    let first = run_wind(&value, 7, 250_000_000);
    let repeated = run_wind(&value, 7, 250_000_000);
    let other = run_wind(&value, 8, 250_000_000);

    assert_eq!(first, repeated);
    assert_ne!(first, other);
    assert_eq!(wind(&value, 250_000_000), run_wind(&value, 0, 250_000_000));
}

#[test]
fn timing_jitter_is_repeatable_and_separates_runs() {
    let mut value = condition(42);
    value.timing = TimingCondition {
        estimate_delay_ns: 2_000_000,
        update_jitter: DelayJitter::SampleAndHold {
            maximum_delay_ns: 3_000_000,
            interval_ns: 100_000_000,
        },
    };

    let first = source_delay(&value, 7, 250_000_000);
    let repeated = source_delay(&value, 7, 250_000_000);
    let other_interval = source_delay(&value, 7, 350_000_000);
    let other_run = source_delay(&value, 8, 250_000_000);

    assert_eq!(first, repeated);
    assert!((2_000_000..=5_000_000).contains(&first));
    assert!(first != other_interval || first != other_run);
}

#[test]
fn a_gust_uses_rise_hold_and_fall_intervals() {
    let mut value = condition(1);
    value.wind.turbulence = TurbulenceModel::None;
    assert!((wind(&value, 0).speed_mps - 5.0).abs() < 1e-9);
    assert!((wind(&value, 1_500_000_000).speed_mps - 6.5).abs() < 1e-9);
    assert!((wind(&value, 2_500_000_000).speed_mps - 8.0).abs() < 1e-9);
    assert!((wind(&value, 3_500_000_000).speed_mps - 6.5).abs() < 1e-9);
    assert!((wind(&value, 4_000_000_000).speed_mps - 5.0).abs() < 1e-9);
}

#[test]
fn canonical_identity_changes_with_the_seed() {
    let first = condition(42).canonical_digest().expect("valid condition");
    let second = condition(43).canonical_digest().expect("valid condition");
    assert_ne!(first, second);
}

#[test]
fn validation_rejects_unbounded_weather() {
    let mut value = condition(42);
    value.wind.gusts[0].speed_mps = 100.0;
    assert!(matches!(
        value.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}

#[test]
fn schedule_resolution_rejects_invalid_direct_construction() {
    let mut value = condition(42);
    value.wind.steady.speed_mps = f64::NAN;
    assert!(matches!(
        value.wind_at(0),
        Err(ValidationError::NonFinite { .. })
    ));
    assert!(value.source_delay_ns_for_run(7, 0).is_err());
}

#[test]
fn validation_rejects_unbounded_timing_variations() {
    let mut value = condition(42);
    value.timing.estimate_delay_ns = 100_000_001;
    assert!(matches!(
        value.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}

#[test]
fn strict_json_rejects_unknown_fields() {
    let mut json = serde_json::to_value(condition(42)).expect("condition value");
    json.as_object_mut()
        .expect("condition object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    let bytes = serde_json::to_vec(&json).expect("condition JSON");
    assert!(matches!(
        ConditionSet::from_json(&bytes),
        Err(CodecError::Decode { .. })
    ));
}

#[test]
fn all_zero_gust_duration_is_rejected() {
    let mut value = condition(42);
    value.wind.gusts[0].rise_ns = 0;
    value.wind.gusts[0].hold_ns = 0;
    value.wind.gusts[0].fall_ns = 0;
    assert_eq!(
        value.validate(),
        Err(ValidationError::ZeroDuration {
            field: "condition_set.wind.gusts[0].duration".to_owned(),
        })
    );
}

#[test]
fn gust_end_time_overflow_is_rejected() {
    let mut value = condition(42);
    value.wind.gusts[0].start_ns = u64::MAX;
    value.wind.gusts[0].rise_ns = 1;
    value.wind.gusts[0].hold_ns = 0;
    value.wind.gusts[0].fall_ns = 0;
    assert_eq!(
        value.validate(),
        Err(ValidationError::TimeOverflow {
            field: "condition_set.wind.gusts[0].end_ns".to_owned(),
        })
    );
}

#[test]
fn zero_rise_and_fall_form_an_exact_pulse() {
    let mut value = condition(42);
    value.wind.turbulence = TurbulenceModel::None;
    value.wind.gusts[0] = GustEvent {
        start_ns: 1_000,
        rise_ns: 0,
        hold_ns: 1_000,
        fall_ns: 0,
        speed_mps: 3.0,
        direction_deg: 270.0,
    };
    let speeds: Vec<_> = [999, 1_000, 1_999, 2_000]
        .map(|time| wind(&value, time).speed_mps)
        .into_iter()
        .collect();
    assert_eq!(speeds, vec![5.0, 8.0, 8.0, 5.0]);
}

#[test]
fn zero_hold_forms_a_continuous_triangle() {
    let mut value = condition(42);
    value.wind.turbulence = TurbulenceModel::None;
    value.wind.gusts[0] = GustEvent {
        start_ns: 1_000,
        rise_ns: 1_000,
        hold_ns: 0,
        fall_ns: 1_000,
        speed_mps: 3.0,
        direction_deg: 270.0,
    };
    assert!((wind(&value, 1_999).speed_mps - 7.997).abs() < 1e-9);
    assert!((wind(&value, 2_000).speed_mps - 8.0).abs() < 1e-9);
    assert!((wind(&value, 2_001).speed_mps - 7.997).abs() < 1e-9);
}

#[test]
fn invalid_zero_turbulence_interval_is_total() {
    let mut value = condition(42);
    value.wind.gusts.clear();
    value.wind.turbulence = TurbulenceModel::BandLimitedNoise {
        amplitude_mps: 1.0,
        knot_interval_ns: 0,
    };
    assert!(matches!(
        value.validate(),
        Err(ValidationError::ZeroDuration { .. })
    ));
    assert!(value.wind_at(1_000).is_err());
}

#[test]
fn invalid_zero_jitter_values_are_total() {
    let mut value = condition(42);
    value.timing = TimingCondition {
        estimate_delay_ns: 2_000_000,
        update_jitter: DelayJitter::SampleAndHold {
            maximum_delay_ns: 3_000_000,
            interval_ns: 0,
        },
    };
    assert!(matches!(
        value.validate(),
        Err(ValidationError::ZeroDuration { .. })
    ));
    assert!(value.source_delay_ns_for_run(7, 250_000_000).is_err());

    value.timing.update_jitter = DelayJitter::SampleAndHold {
        maximum_delay_ns: 0,
        interval_ns: 250_000_000,
    };
    assert!(matches!(
        value.validate(),
        Err(ValidationError::ZeroDuration { .. })
    ));
    assert!(value.source_delay_ns_for_run(7, 250_000_000).is_err());
}

#[test]
fn fixed_condition_timeline_matches_the_golden_values_after_reset() {
    let value = ConditionSet {
        schema_version: CONDITION_SET_SCHEMA_VERSION,
        id: "golden-timeline".to_owned(),
        revision: 1,
        seed: 42,
        wind: WindCondition {
            steady: HorizontalWind {
                speed_mps: 2.0,
                direction_deg: 0.0,
            },
            gusts: vec![GustEvent {
                start_ns: 1_000_000_000,
                rise_ns: 1_000_000_000,
                hold_ns: 1_000_000_000,
                fall_ns: 1_000_000_000,
                speed_mps: 4.0,
                direction_deg: 0.0,
            }],
            turbulence: TurbulenceModel::None,
        },
        timing: TimingCondition {
            estimate_delay_ns: 2_000_000,
            update_jitter: DelayJitter::SampleAndHold {
                maximum_delay_ns: 3_000_000,
                interval_ns: 250_000_000,
            },
        },
        sensor: SensorCondition::nominal(),
        actuator: ActuatorCondition::nominal(),
        controller_initialization: ControllerInitializationCondition::nominal(),
    };
    let expected = [
        (0, 2.0, 4_246_789),
        (250_000_000, 2.0, 2_712_636),
        (500_000_000, 2.0, 2_980_386),
        (750_000_000, 2.0, 4_221_038),
        (1_000_000_000, 2.0, 3_671_211),
        (1_500_000_000, 4.0, 4_303_391),
        (2_000_000_000, 6.0, 3_282_861),
        (2_500_000_000, 6.0, 2_425_430),
        (3_000_000_000, 6.0, 4_647_388),
        (3_500_000_000, 4.0, 2_280_806),
        (4_000_000_000, 2.0, 3_694_917),
    ];
    assert_eq!(
        value
            .canonical_digest()
            .expect("golden condition")
            .to_string(),
        "3b7493b39b2075327ebe650e480e1bbe7d76d4fed6839f38af2bdb5268cf4069"
    );

    let first: Vec<_> = expected
        .iter()
        .map(|(time, _, _)| (run_wind(&value, 7, *time), source_delay(&value, 7, *time)))
        .collect();
    let reset: Vec<_> = expected
        .iter()
        .map(|(time, _, _)| (run_wind(&value, 7, *time), source_delay(&value, 7, *time)))
        .collect();
    assert_eq!(first, reset);

    for ((wind, delay), (_, speed, expected_delay)) in first.iter().zip(expected) {
        assert!((wind.speed_mps - speed).abs() < 1e-9);
        assert!((wind.north_mps + speed).abs() < 1e-9);
        assert!(wind.east_mps.abs() < 1e-9);
        assert_eq!(*delay, expected_delay);
    }
}

#[test]
fn seeded_turbulence_matches_fixed_vector_samples() {
    let value = condition(42);
    let expected = [
        (0, 0.314_943_760_183_277_5, 4.863_281_615_858_67),
        (100_000_000, 0.099_427_857_735_345_15, 4.938_501_265_056_212),
        (
            200_000_000,
            -0.116_088_044_712_587_21,
            5.013_720_914_253_755,
        ),
        (250_000_000, -0.034_061_976_899_483_91, 5.026_713_109_005_16),
        (400_000_000, 0.212_016_226_539_826, 5.065_689_693_259_377),
    ];
    for (time, north, east) in expected {
        let sample = run_wind(&value, 7, time);
        assert!((sample.north_mps - north).abs() < 1e-12);
        assert!((sample.east_mps - east).abs() < 1e-12);
    }
}
