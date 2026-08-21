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

fn digest_profile() -> FlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = "digest-baseline".to_owned();
    profile.mode = crate::FeelMode::Balanced;
    for response in [
        &mut profile.horizontal,
        &mut profile.vertical,
        &mut profile.yaw,
    ] {
        response.curve.expo = 0.2;
        response.neutral.active_enter = 0.08;
        response.neutral.active_exit = 0.04;
        response.dynamics = AxisDynamics {
            apply_accel: 2.0,
            release_accel: 4.0,
            apply_jerk: 8.0,
            release_jerk: 16.0,
        };
    }
    profile.direct.tilt_rate_rps = 2.0;
    profile.direct.tilt_accel_rps2 = 8.0;
    profile.direct.thrust_rate_per_s = 2.0;
    profile.direct.thrust_accel_per_s2 = 8.0;
    profile.hold.max_speed_mps = 0.2;
    profile.hold.max_accel_mps2 = 0.4;
    profile.hold.require_accel = false;
    profile.hold.stable_dwell_ms = 200;
    profile
}

macro_rules! assert_axis_fields_change_digest {
    ($assertion:ident, $axis:ident) => {
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.curve.expo += 0.01);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.neutral.active_enter += 0.01);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.neutral.active_exit += 0.01);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.dynamics.apply_accel += 0.1);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.dynamics.release_accel += 0.1);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.dynamics.apply_jerk += 0.1);
        $assertion!(|p: &mut FlightFeelProfile| p.$axis.dynamics.release_jerk += 0.1);
    };
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
    let base = ValidatedFlightFeelProfile::new(digest_profile()).expect("base profile");
    let base_digest = FeelDigest::calculate(&base).expect("base digest");
    macro_rules! assert_field_changes_digest {
        ($change:expr) => {{
            let mut changed = base.profile().clone();
            $change(&mut changed);
            let changed = ValidatedFlightFeelProfile::new(changed).expect("changed profile");
            assert_ne!(
                base_digest,
                FeelDigest::calculate(&changed).expect("changed digest")
            );
        }};
    }
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.profile_id = "digest-other".into());
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.mode = crate::FeelMode::Agile);
    assert_field_changes_digest!(
        |p: &mut FlightFeelProfile| p.envelope.horizontal_speed_mps += 0.1
    );
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.envelope.vertical_speed_mps += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.envelope.yaw_rate_rps += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.envelope.direct_tilt_rad += 0.1);
    assert_field_changes_digest!(
        |p: &mut FlightFeelProfile| p.envelope.direct_hover_thrust += 0.01
    );
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.envelope.direct_min_thrust += 0.01);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.envelope.takeoff_input += 0.01);
    assert_axis_fields_change_digest!(assert_field_changes_digest, horizontal);
    assert_axis_fields_change_digest!(assert_field_changes_digest, vertical);
    assert_axis_fields_change_digest!(assert_field_changes_digest, yaw);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.direct.tilt_rate_rps += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.direct.tilt_accel_rps2 += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.direct.thrust_rate_per_s += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.direct.thrust_accel_per_s2 += 0.1);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.hold.max_speed_mps += 0.01);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.hold.max_accel_mps2 += 0.01);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.hold.require_accel = true);
    assert_field_changes_digest!(|p: &mut FlightFeelProfile| p.hold.stable_dwell_ms += 1);
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

#[test]
fn release_never_increases_command_magnitude() {
    let limits = AxisDynamics {
        apply_accel: 1.0,
        release_accel: 2.0,
        apply_jerk: 4.0,
        release_jerk: 8.0,
    };
    let mut axis = JerkLimitedAxis::default();
    for _ in 0..30 {
        assert!(axis.step(3.0, 0.02, DemandPhase::Apply, limits).is_finite());
    }
    let mut prior = axis.value().abs();
    for _ in 0..100 {
        let value = axis.step(0.0, 0.02, DemandPhase::Release, limits);
        assert!(value.abs() <= prior + 1e-6, "{value} followed {prior}");
        prior = value.abs();
    }
    assert_eq!(axis.value(), 0.0);
}

#[test]
fn a_reversed_target_does_not_move_farther_from_the_new_target() {
    let limits = AxisDynamics {
        apply_accel: 2.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 20.0,
    };
    let mut axis = JerkLimitedAxis::default();
    for _ in 0..20 {
        assert!(axis.step(1.0, 0.02, DemandPhase::Apply, limits).is_finite());
    }
    let before = (axis.value() + 1.0).abs();
    let after = (axis.step(-1.0, 0.02, DemandPhase::Apply, limits) + 1.0).abs();
    assert!(after <= before);
}

#[test]
fn release_phase_starts_toward_neutral_with_release_limits() {
    let limits = AxisDynamics {
        apply_accel: 2.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 20.0,
    };
    let mut axis = JerkLimitedAxis::default();
    for _ in 0..20 {
        assert!(axis.step(1.0, 0.02, DemandPhase::Apply, limits).is_finite());
    }
    let before = axis.value();

    let after = axis.step(0.0, 0.02, DemandPhase::Release, limits);

    assert!(after <= before);
    assert!(axis.rate() <= 0.0);
    assert!(axis.rate().abs() <= limits.release_jerk * 0.02 + f32::EPSILON);
    assert!(axis.rate().abs() <= limits.release_accel);
}

#[test]
fn invalid_time_steps_and_limits_keep_finite_state() {
    let limits = AxisDynamics {
        apply_accel: 2.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 20.0,
    };
    for dt in [0.0, -0.1, f32::NAN, f32::INFINITY] {
        let mut axis = JerkLimitedAxis::default();
        assert_eq!(axis.step(1.0, dt, DemandPhase::Apply, limits), 0.0);
        assert!(axis.rate().is_finite());
    }
    let mut axis = JerkLimitedAxis::default();
    let mut bad = limits;
    bad.apply_jerk = f32::NAN;
    assert_eq!(axis.step(1.0, 0.02, DemandPhase::Apply, bad), 0.0);
}

#[test]
fn equal_wall_time_is_nearly_invariant_to_sample_rate() {
    let limits = AxisDynamics {
        apply_accel: 2.0,
        release_accel: 4.0,
        apply_jerk: 10.0,
        release_jerk: 20.0,
    };
    let run = |hz: usize| {
        let mut axis = JerkLimitedAxis::default();
        let dt = 1.0 / hz as f32;
        for _ in 0..hz {
            assert!(axis.step(1.0, dt, DemandPhase::Apply, limits).is_finite());
        }
        axis.value()
    };
    let at_30 = run(30);
    for hz in [60, 120] {
        assert!((run(hz) - at_30).abs() < 0.03, "rate {hz}");
    }
}

#[test]
fn validation_rejects_an_unreachable_takeoff_threshold() {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.envelope.takeoff_input = 1.0;
    assert!(matches!(
        ValidatedFlightFeelProfile::new(profile),
        Err(ValidationError::FieldOutOfRange {
            field: "envelope.takeoff_input"
        })
    ));
}

#[test]
fn validation_rejects_weaker_release_dynamics() {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.horizontal.dynamics.release_accel = 1.0;
    assert!(matches!(
        ValidatedFlightFeelProfile::new(profile),
        Err(ValidationError::InvalidOrder { .. })
    ));
}
