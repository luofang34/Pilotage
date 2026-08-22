#![allow(clippy::expect_used, clippy::panic)]

use super::*;

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
    }
}

#[test]
fn the_same_seed_produces_the_same_schedule() {
    let first = condition(42);
    let second = condition(42);
    let first_samples: Vec<_> = (0..100)
        .map(|index| first.wind_at(index * 50_000_000))
        .collect();
    let second_samples: Vec<_> = (0..100)
        .map(|index| second.wind_at(index * 50_000_000))
        .collect();
    assert_eq!(first_samples, second_samples);
}

#[test]
fn a_different_seed_changes_the_turbulence_schedule() {
    assert_ne!(
        condition(42).wind_at(250_000_000),
        condition(43).wind_at(250_000_000)
    );
}

#[test]
fn a_run_seed_is_repeatable_and_separates_repetitions() {
    let value = condition(42);
    let first = value.wind_at_for_run(7, 250_000_000);
    let repeated = value.wind_at_for_run(7, 250_000_000);
    let other = value.wind_at_for_run(8, 250_000_000);

    assert_eq!(first, repeated);
    assert_ne!(first, other);
    assert_eq!(
        value.wind_at(250_000_000),
        value.wind_at_for_run(0, 250_000_000)
    );
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

    let first = value.source_delay_ns_for_run(7, 250_000_000);
    let repeated = value.source_delay_ns_for_run(7, 250_000_000);
    let other_interval = value.source_delay_ns_for_run(7, 350_000_000);
    let other_run = value.source_delay_ns_for_run(8, 250_000_000);

    assert_eq!(first, repeated);
    assert!((2_000_000..=5_000_000).contains(&first));
    assert!(first != other_interval || first != other_run);
}

#[test]
fn a_gust_uses_rise_hold_and_fall_intervals() {
    let mut value = condition(1);
    value.wind.turbulence = TurbulenceModel::None;
    assert!((value.wind_at(0).speed_mps - 5.0).abs() < 1e-9);
    assert!((value.wind_at(1_500_000_000).speed_mps - 6.5).abs() < 1e-9);
    assert!((value.wind_at(2_500_000_000).speed_mps - 8.0).abs() < 1e-9);
    assert!((value.wind_at(3_500_000_000).speed_mps - 6.5).abs() < 1e-9);
    assert!((value.wind_at(4_000_000_000).speed_mps - 5.0).abs() < 1e-9);
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
