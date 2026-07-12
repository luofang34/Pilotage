#![allow(clippy::expect_used, clippy::panic)]

//! NAV-01 resolution proofs: heading is the independent sample, never
//! quaternion yaw; every HSI angle presents in one reference or
//! visibly degrades; conversion happens only through a usable
//! variation sample.

use core::f32::consts::PI;

use super::resolve;
use super::tests::flying_state;
use crate::aircraft::{AircraftState, Stamped};
use crate::heading::{HeadingReference, HeadingSample, MagneticVariation, VariationSourceId};
use crate::signal::{FreshnessPolicy, SignalStatus};
use pilotage_frames::Quat;

const DEG: f32 = PI / 180.0;

fn with_heading(reference: HeadingReference, heading_rad: f32) -> AircraftState {
    let mut state = flying_state();
    state.heading = Stamped {
        data: Some(HeadingSample {
            heading_rad,
            reference,
        }),
        age_ms: Some(20.0),
    };
    state.valid.heading = true;
    state
}

fn with_variation(mut state: AircraftState, east_deg: f32) -> AircraftState {
    state.variation = Stamped {
        data: Some(MagneticVariation {
            east_positive_rad: east_deg * DEG,
            source: VariationSourceId(1),
        }),
        age_ms: Some(50.0),
    };
    state.valid.variation = true;
    state
}

#[test]
fn missing_heading_resolves_missing_even_with_a_valid_attitude() {
    let p = resolve(&flying_state(), &FreshnessPolicy::default());
    assert_eq!(p.heading.value_rad.status, SignalStatus::Missing);
    assert_eq!(p.heading.reference, HeadingReference::Unknown);
    assert_eq!(
        p.heading.value_rad.value, 0.0,
        "no plausible frozen heading may remain"
    );
}

#[test]
fn heading_is_independent_of_attitude_through_the_vertical() {
    for pitch_deg in [89.0f32, 90.0, 91.0, 180.0] {
        let mut state = with_heading(HeadingReference::SimLocalTrue, 1.0);
        let half = pitch_deg * DEG / 2.0;
        // Pure pitch rotation: ill-conditioned yaw at/beyond vertical.
        state.attitude.data = state.attitude.data.map(|mut att| {
            att.quat = Quat {
                w: libm::cosf(half),
                x: 0.0,
                y: libm::sinf(half),
                z: 0.0,
            };
            att
        });
        let p = resolve(&state, &FreshnessPolicy::default());
        assert_eq!(p.heading.value_rad.status, SignalStatus::Valid);
        assert!(
            (p.heading.value_rad.value - 1.0).abs() < 1e-6,
            "independent heading must not bend at pitch {pitch_deg}"
        );
    }
}

#[test]
fn unknown_reference_fails_the_heading() {
    let state = with_heading(HeadingReference::from_u8(9), 1.0);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading.value_rad.status, SignalStatus::Failed);
    assert_eq!(p.heading.reference, HeadingReference::Unknown);
}

#[test]
fn undeclared_validity_fails_the_heading() {
    let mut state = with_heading(HeadingReference::True, 1.0);
    state.valid.heading = false;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading.value_rad.status, SignalStatus::Failed);
}

#[test]
fn heading_value_wraps_into_one_turn() {
    let state = with_heading(HeadingReference::SimLocalTrue, 370.0 * DEG);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!((p.heading.value_rad.value - 10.0 * DEG).abs() < 1e-4);
}

#[test]
fn magnetic_display_without_variation_degrades_true_quantities() {
    // Track and wind are NED-derived (true); a magnetic rose must not
    // mix them in unconverted.
    let state = with_heading(HeadingReference::Magnetic, 1.0);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.track_rad.status, SignalStatus::Failed);
}

#[test]
fn magnetic_display_with_variation_converts_the_track() {
    let mut state = with_variation(with_heading(HeadingReference::Magnetic, 1.0), 2.0);
    // Velocity due north: true track 0° reads magnetic 358° under 2°E.
    if let Some(kin) = state.kinematics.data.as_mut() {
        kin.vel_ned_mps = [20.0, 0.0, 0.0];
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.track_rad.status, SignalStatus::Valid);
    assert!(
        (p.track_rad.value - 358.0 * DEG).abs() < 1e-4,
        "got {}",
        p.track_rad.value
    );
}

#[test]
fn stale_variation_refuses_conversion() {
    let mut state = with_variation(with_heading(HeadingReference::Magnetic, 1.0), 2.0);
    state.variation.age_ms = Some(1.0e9);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.track_rad.status, SignalStatus::Failed);
}

#[test]
fn sim_true_display_presents_true_quantities_unchanged() {
    let mut state = with_heading(HeadingReference::SimLocalTrue, 1.0);
    if let Some(kin) = state.kinematics.data.as_mut() {
        kin.vel_ned_mps = [0.0, 20.0, 0.0];
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.track_rad.status, SignalStatus::Valid);
    assert!((p.track_rad.value - 90.0 * DEG).abs() < 1e-4);
}

// ---- Review: every displayed angle is typed and rose-coherent -----------

fn with_bug(mut state: AircraftState, bug_deg: f32, reference: HeadingReference) -> AircraftState {
    state.selections.heading_bug_rad = bug_deg * DEG;
    state.selections.heading_bug_reference = reference;
    state
}

fn with_course(
    mut state: AircraftState,
    course_deg: f32,
    reference: HeadingReference,
) -> AircraftState {
    state.nav = Stamped {
        data: Some(crate::aircraft::NavData {
            source: crate::aircraft::NavSource::Gps,
            course_rad: course_deg * DEG,
            cdi_dots: 0.5,
            fromto: crate::aircraft::NavFromTo::To,
            vdev_dots: None,
            dist_nm: None,
            course_reference: reference,
        }),
        age_ms: Some(20.0),
    };
    state
}

#[test]
fn true_bug_on_a_magnetic_rose_fails_without_variation() {
    let state = with_bug(
        with_heading(HeadingReference::Magnetic, 1.0),
        90.0,
        HeadingReference::True,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading_bug_rose_rad.status, SignalStatus::Failed);
    assert_eq!(p.heading_bug_rose_rad.value, 0.0);
}

#[test]
fn true_course_on_a_magnetic_rose_fails_without_variation() {
    let state = with_course(
        with_heading(HeadingReference::Magnetic, 1.0),
        90.0,
        HeadingReference::True,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.nav.course_rose_rad.status, SignalStatus::Failed);
    assert_eq!(
        p.nav.status,
        SignalStatus::Valid,
        "one incompatible quantity fails alone, not the nav group"
    );
}

#[test]
fn valid_variation_converts_bug_and_course_onto_the_magnetic_rose() {
    let state = with_variation(
        with_course(
            with_bug(
                with_heading(HeadingReference::Magnetic, 1.0),
                90.0,
                HeadingReference::True,
            ),
            180.0,
            HeadingReference::True,
        ),
        2.0,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading_bug_rose_rad.status, SignalStatus::Valid);
    assert!((p.heading_bug_rose_rad.value - 88.0 * DEG).abs() < 1e-4);
    assert_eq!(p.nav.course_rose_rad.status, SignalStatus::Valid);
    assert!((p.nav.course_rose_rad.value - 178.0 * DEG).abs() < 1e-4);
}

#[test]
fn numerically_identical_angles_in_incompatible_references_never_pass_raw() {
    // Bug 090 TRUE on a 090-showing MAGNETIC rose under 2E variation:
    // the drawn bug is 088, never the raw 090.
    let state = with_variation(
        with_bug(
            with_heading(HeadingReference::Magnetic, 90.0 * DEG),
            90.0,
            HeadingReference::True,
        ),
        2.0,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert!((p.heading_bug_rose_rad.value - 88.0 * DEG).abs() < 1e-4);
    assert!((p.heading_bug_rose_rad.value - p.selections.heading_bug_rad).abs() > 1e-3);
}

#[test]
fn unknown_bug_or_course_reference_fails_that_quantity() {
    let state = with_course(
        with_bug(
            with_heading(HeadingReference::SimLocalTrue, 1.0),
            45.0,
            HeadingReference::Unknown,
        ),
        60.0,
        HeadingReference::Unknown,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading_bug_rose_rad.status, SignalStatus::Failed);
    assert_eq!(p.nav.course_rose_rad.status, SignalStatus::Failed);
}

#[test]
fn every_displayed_angle_agrees_on_the_rose_reference() {
    // Magnetic rose, valid 2E variation, all true-north inputs: every
    // presented angle is the true value minus variation — one coherent
    // reference across heading, track, bug, and course.
    let mut state = with_variation(
        with_course(
            with_bug(
                with_heading(HeadingReference::Magnetic, 100.0 * DEG),
                90.0,
                HeadingReference::True,
            ),
            180.0,
            HeadingReference::True,
        ),
        2.0,
    );
    if let Some(kin) = state.kinematics.data.as_mut() {
        kin.vel_ned_mps = [0.0, 20.0, 0.0];
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.heading.reference, HeadingReference::Magnetic);
    for (name, sig, true_deg) in [
        ("track", p.track_rad, 90.0f32),
        ("bug", p.heading_bug_rose_rad, 90.0),
        ("course", p.nav.course_rose_rad, 180.0),
    ] {
        assert_eq!(sig.status, SignalStatus::Valid, "{name}");
        assert!(
            (sig.value - (true_deg - 2.0) * DEG).abs() < 1e-4,
            "{name} must present magnetic: {}",
            sig.value
        );
    }
}
