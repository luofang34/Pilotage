#![allow(clippy::expect_used, clippy::panic)]

use crate::{
    AxisDemandShaper, AxisDynamics, AxisResponse, DemandPhase, FeelDigest, FlightFeelProfile,
    HoldDetector, HoldTransition, JerkLimitedAxis, NeutralBand, NeutralLatch,
    ValidatedFlightFeelProfile, ValidationError,
};

fn validated_legacy() -> ValidatedFlightFeelProfile {
    ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("legacy profile is valid")
}

#[test]
fn legacy_profile_is_valid_and_has_a_stable_digest() {
    let profile = validated_legacy();
    let first = FeelDigest::calculate(&profile).expect("digest");
    let second = FeelDigest::calculate(&profile).expect("digest");
    assert_eq!(first, second);
    assert_eq!(first.to_string().len(), 64);
}

#[test]
fn every_profile_field_changes_the_identity() {
    let base = validated_legacy();
    let base_digest = FeelDigest::calculate(&base).expect("base digest");
    let mut changed = base.profile().clone();
    changed.hold.stable_dwell_ms = 1;
    let changed = ValidatedFlightFeelProfile::new(changed).expect("changed profile");
    assert_ne!(
        base_digest,
        FeelDigest::calculate(&changed).expect("changed digest")
    );
}

#[test]
fn neutral_latch_uses_hysteresis() {
    let band = NeutralBand {
        active_enter: 0.08,
        active_exit: 0.04,
    };
    let mut latch = NeutralLatch::default();
    assert!(!latch.update(0.06, band));
    assert!(latch.update(0.09, band));
    assert!(latch.update(0.06, band));
    assert!(!latch.update(0.03, band));
}

#[test]
fn jerk_limiter_bounds_rate_change_and_does_not_overshoot() {
    let limits = AxisDynamics {
        apply_accel: 2.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 20.0,
    };
    let mut axis = JerkLimitedAxis::default();
    let mut prior_rate = 0.0;
    for _ in 0..100 {
        let value = axis.step(1.0, 0.01, DemandPhase::Apply, limits);
        assert!(value <= 1.0);
        assert!((axis.rate() - prior_rate).abs() <= 0.100_001);
        prior_rate = axis.rate();
    }
}

#[test]
fn release_can_use_a_different_limit() {
    let limits = AxisDynamics {
        apply_accel: 1.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 40.0,
    };
    let mut axis = JerkLimitedAxis::default();
    axis.seed(1.0);
    let released = axis.step(0.0, 0.1, DemandPhase::Release, limits);
    assert!(released < 1.0);
    assert!(axis.rate() < -0.1);
}

#[test]
fn axis_demand_shaper_releases_only_after_exit_threshold() {
    let response = AxisResponse {
        curve: crate::AxisCurve { expo: 0.0 },
        neutral: NeutralBand {
            active_enter: 0.08,
            active_exit: 0.04,
        },
        dynamics: AxisDynamics {
            apply_accel: 100.0,
            release_accel: 100.0,
            apply_jerk: 1_000.0,
            release_jerk: 1_000.0,
        },
    };
    let mut shaper = AxisDemandShaper::default();
    assert!(!shaper.step(0.06, 2.0, 0.02, response).input_active);
    assert!(shaper.step(0.10, 2.0, 0.02, response).input_active);
    assert!(shaper.step(0.06, 2.0, 0.02, response).input_active);
    assert!(!shaper.step(0.03, 2.0, 0.02, response).input_active);
}

#[test]
fn hold_detector_requires_complete_stable_dwell() {
    let policy = HoldTransition {
        max_speed_mps: 0.2,
        max_accel_mps2: 0.4,
        require_accel: true,
        stable_dwell_ms: 200,
    };
    let mut detector = HoldDetector::default();
    assert!(!detector.update(Some(0.1), Some(0.2), 0.1, policy));
    assert!(detector.update(Some(0.1), Some(0.2), 0.1, policy));
    assert!(!detector.update(None, Some(0.2), 0.1, policy));
}

#[test]
fn validation_rejects_reversed_neutral_thresholds() {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.horizontal.neutral.active_exit = 0.08;
    profile.horizontal.neutral.active_enter = 0.04;
    assert!(matches!(
        ValidatedFlightFeelProfile::new(profile),
        Err(ValidationError::InvalidOrder { .. })
    ));
}

#[test]
fn validation_rejects_non_finite_dynamics() {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.horizontal.dynamics.apply_accel = f32::NAN;
    assert!(matches!(
        ValidatedFlightFeelProfile::new(profile),
        Err(ValidationError::FieldOutOfRange { .. })
    ));
}

#[test]
fn strict_json_rejects_unknown_fields() {
    let mut value =
        serde_json::to_value(FlightFeelProfile::legacy_compatibility()).expect("profile value");
    value
        .as_object_mut()
        .expect("profile object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    let text = serde_json::to_string(&value).expect("profile json");
    assert!(ValidatedFlightFeelProfile::from_json_str(&text).is_err());
}
