//! The deviation-to-dots profile: fly-to signs, unit conversions, and
//! the absent-not-centered contract.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_state::{IdentStr, NavFromTo};

use super::{LATERAL_M_PER_DOT, M_PER_NM, VDEV_M_PER_DOT, nav_display_state};
use crate::nav_guidance::{Guidance, NavSnapshot};

fn snapshot(guidance: Guidance) -> NavSnapshot {
    NavSnapshot {
        guidance,
        age_ms: 120.0,
    }
}

fn guidance() -> Guidance {
    Guidance {
        to_ident: IdentStr::new("WPT-2").expect("valid"),
        from_ident: IdentStr::new("KMRY").expect("valid"),
        course_rad: 0.6,
        lateral_deviation_m: 25.0,
        vertical_deviation_m: 8.0,
        distance_to_waypoint_m: 1852.0,
        leg_index: 1,
        waypoint_count: 4,
        solution_quality: 0,
    }
}

#[test]
fn absent_guidance_is_absent_not_centered() {
    assert!(nav_display_state(None).is_none());
    let mut unusable = guidance();
    unusable.solution_quality = 2;
    assert!(nav_display_state(Some(&snapshot(unusable))).is_none());
    let mut unknown_quality = guidance();
    unknown_quality.solution_quality = 7;
    assert!(nav_display_state(Some(&snapshot(unknown_quality))).is_none());
}

#[test]
fn fly_to_signs_and_units_convert_exactly() {
    let stamped = nav_display_state(Some(&snapshot(guidance()))).expect("displays");
    let nav = stamped.data.expect("group present");
    // Ownship 25 m RIGHT of course: the course line is LEFT, so the
    // deflection is one dot NEGATIVE and flying toward the bar closes
    // the error.
    assert!((nav.cdi_dots + 25.0 / LATERAL_M_PER_DOT).abs() < 1e-6);
    // Ownship 8 m ABOVE profile: the profile is LOWER on the display,
    // one dot POSITIVE — the axes disagree on which way is up.
    assert!((nav.vdev_dots.expect("constrained") - 8.0 / VDEV_M_PER_DOT).abs() < 1e-6);
    assert!((nav.dist_nm.expect("distance") - 1852.0 / M_PER_NM).abs() < 1e-6);
    assert_eq!(nav.fromto, NavFromTo::To);
    assert_eq!(nav.to_ident.as_str(), "WPT-2");
    assert_eq!(stamped.age_ms, Some(120.0));
}

#[test]
fn untracked_lateral_clears_the_flag_with_a_finite_zero() {
    let mut untracked = guidance();
    untracked.lateral_deviation_m = f32::NAN;
    let stamped = nav_display_state(Some(&snapshot(untracked))).expect("displays");
    let nav = stamped.data.expect("group present");
    assert_eq!(nav.fromto, NavFromTo::Off);
    assert_eq!(nav.cdi_dots, 0.0);
    // The still-valid course and distance survive.
    assert!((nav.course_rad - 0.6).abs() < 1e-6);
}

#[test]
fn unconstrained_vertical_stays_absent() {
    let mut flat = guidance();
    flat.vertical_deviation_m = f32::NAN;
    let stamped = nav_display_state(Some(&snapshot(flat))).expect("displays");
    assert_eq!(stamped.data.expect("group").vdev_dots, None);
}
