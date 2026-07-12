#![allow(clippy::expect_used, clippy::panic)]

//! ALT-01 resolution proofs: datum-qualified altitude, no fallback,
//! and full-reference-identity selection compatibility.

use super::tests::flying_state;
use super::{FreshnessPolicy, SignalStatus, resolve};
use crate::altitude::{AltitudeClass, AltitudeDeclaration, GeoidModelId, OriginId};

#[test]
fn local_relative_is_labelled_rel_and_reads_1000_ft() {
    let p = resolve(&flying_state(), &FreshnessPolicy::default());
    assert_eq!(p.altitude.class, AltitudeClass::LocalRelative);
    assert!((p.altitude.value_ft.value - 1000.0).abs() < 0.5);
    assert_eq!(p.altitude.class.label(), "REL");
}

#[test]
fn baro_class_without_a_source_fails_and_never_uses_ned() {
    let mut state = flying_state();
    state.altitude.reference_class = AltitudeClass::BaroIndicated;
    // NED says 1000 ft, but no barometric sample exists.
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
    assert_eq!(
        p.altitude.value_ft.value, 0.0,
        "no NED fallback may leak into a barometric tape"
    );
}

#[test]
fn baro_class_with_sample_and_setting_displays_the_sample() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::BaroIndicated,
        sample_m: Some(457.2),
        ..AltitudeDeclaration::default()
    };
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Valid);
    assert!((p.altitude.value_ft.value - 1500.0).abs() < 0.5);
    assert_eq!(p.altitude.class, AltitudeClass::BaroIndicated);
}

#[test]
fn baro_class_without_applied_setting_fails() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::BaroIndicated,
        sample_m: Some(457.2),
        ..AltitudeDeclaration::default()
    };
    if let Some(air) = state.air.data.as_mut() {
        air.baro_setting_hpa = None;
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
}

#[test]
fn geometric_msl_requires_a_declared_model() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::GeometricMsl,
        sample_m: Some(500.0),
        geoid_model: GeoidModelId::UNDECLARED,
        origin: OriginId(0),
    };
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
    state.altitude.geoid_model = GeoidModelId(1);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Valid);
    assert_eq!(p.altitude.class.label(), "MSL");
}

#[test]
fn unknown_reference_class_fails_closed() {
    let mut state = flying_state();
    state.altitude.reference_class = AltitudeClass::from_u8(9);
    state.altitude.sample_m = Some(500.0);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.class, AltitudeClass::Unknown);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
    assert!(!p.altitude.bug_compatible);
}

#[test]
fn below_origin_altitude_stays_correct_and_negative() {
    let mut state = flying_state();
    if let Some(kin) = state.kinematics.data.as_mut() {
        kin.pos_ned_m[2] = 152.4;
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!((p.altitude.value_ft.value + 500.0).abs() < 0.5);
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Valid);
}

#[test]
fn bug_requires_matching_class_before_identity_is_even_considered() {
    let mut state = flying_state();
    state.selections.altitude_sel_m = Some(304.8);
    state.selections.altitude_sel_class = AltitudeClass::LocalRelative;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(p.altitude.bug_compatible);

    // Numerically identical selection, barometric reference: no bug.
    // Class equality is necessary but NEVER sufficient — the
    // identity tests below prove the rest of the rule.
    state.selections.altitude_sel_class = AltitudeClass::BaroIndicated;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(!p.altitude.bug_compatible);
}

#[test]
fn equal_values_from_different_origins_are_incompatible() {
    let mut state = flying_state();
    state.altitude.origin = OriginId(1);
    state.selections.altitude_sel_m = Some(304.8);
    state.selections.altitude_sel_class = AltitudeClass::LocalRelative;
    state.selections.altitude_sel_origin = OriginId(2);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(
        !p.altitude.bug_compatible,
        "same class, same number, different origin: not the same datum"
    );
    state.selections.altitude_sel_origin = OriginId(1);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(p.altitude.bug_compatible);
}

#[test]
fn equal_values_from_different_geoid_models_are_incompatible() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::GeometricMsl,
        sample_m: Some(304.8),
        geoid_model: GeoidModelId(1),
        origin: OriginId(0),
    };
    state.selections.altitude_sel_m = Some(304.8);
    state.selections.altitude_sel_class = AltitudeClass::GeometricMsl;
    state.selections.altitude_sel_model = GeoidModelId(2);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(!p.altitude.bug_compatible, "model A vs model B");

    // An undeclared selection model is an incomplete identity and
    // fails closed even when the displayed model would match zero.
    state.selections.altitude_sel_model = GeoidModelId::UNDECLARED;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(!p.altitude.bug_compatible, "undeclared identity");

    state.selections.altitude_sel_model = GeoidModelId(1);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(p.altitude.bug_compatible);
}

#[test]
fn disputed_baro_setting_suppresses_the_baro_bug() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::BaroIndicated,
        sample_m: Some(457.2),
        ..AltitudeDeclaration::default()
    };
    state.selections.altitude_sel_m = Some(457.2);
    state.selections.altitude_sel_class = AltitudeClass::BaroIndicated;
    state.selections.baro_sel_hpa = Some(1020.0);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(
        !p.altitude.bug_compatible,
        "the applied setting IS the barometric datum; disputed means ambiguous"
    );
    state.selections.baro_sel_hpa = Some(1013.25);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(p.altitude.bug_compatible);
}

#[test]
fn setting_mismatch_is_flagged_and_agreement_is_not() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::BaroIndicated,
        sample_m: Some(457.2),
        ..AltitudeDeclaration::default()
    };
    state.selections.baro_sel_hpa = Some(1013.25);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(!p.altitude.setting_mismatch, "agreement is quiet");

    state.selections.baro_sel_hpa = Some(1020.0);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(p.altitude.setting_mismatch);

    // Mismatch is a barometric concept; REL never flags it.
    state.altitude.reference_class = AltitudeClass::LocalRelative;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!(!p.altitude.setting_mismatch);
}

#[test]
fn non_finite_sample_fails_with_typed_reason() {
    let mut state = flying_state();
    state.altitude = AltitudeDeclaration {
        reference_class: AltitudeClass::Pressure,
        sample_m: Some(f32::NAN),
        ..AltitudeDeclaration::default()
    };
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.value_ft.status, SignalStatus::Failed);
    assert_eq!(
        p.integrity.altitude,
        Some(crate::validate::GroupFault::NonFinite)
    );
}

#[test]
fn origin_identity_is_preserved() {
    let mut state = flying_state();
    state.altitude.origin = OriginId(7);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.altitude.origin, OriginId(7));
}
