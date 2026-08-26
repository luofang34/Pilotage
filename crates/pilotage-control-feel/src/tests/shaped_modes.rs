#![allow(clippy::expect_used, clippy::panic)]

use crate::{
    AxisDemandShaper, AxisResponse, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile,
};

/// The principles the shaped modes are built on, checked rather than described.
///
/// Each of these is a property an operator would feel the loss of, and none of
/// them is visible in a diff of the numbers alone.
const SHAPED: [FeelMode; 3] = [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile];

fn axes(profile: &FlightFeelProfile) -> [AxisResponse; 3] {
    [profile.horizontal, profile.vertical, profile.yaw]
}

#[test]
fn no_axis_of_any_mode_steps() {
    // An unshaped law reaches full demand in one sample. Every bound here is
    // finite and small enough that a step is impossible: the legacy law's
    // 10_000 per second crosses the whole demand range in a tenth of a
    // millisecond, which is a step by any measure a passenger can feel.
    for mode in SHAPED {
        let profile = FlightFeelProfile::shaped(mode);
        for axis in axes(&profile) {
            let d = axis.dynamics;
            for bound in [
                d.apply_accel,
                d.release_accel,
                d.reversal_accel,
                d.apply_jerk,
                d.release_jerk,
                d.reversal_jerk,
            ] {
                assert!(bound.is_finite() && bound > 0.0, "{mode:?} bound {bound}");
                assert!(bound < 1_000.0, "{mode:?} bound {bound} is a step");
            }
        }
    }
}

#[test]
fn letting_go_is_never_slower_than_asking() {
    // Letting go is how an operator stops asking. A release that lagged the
    // apply would mean the vehicle took longer to stop commanding than it
    // took to start, which is the one ordering a control law must not have
    // however comfortable it reads.
    //
    // A reversal is a correction: no slower than a fresh command, and no
    // quicker than a release.
    for mode in SHAPED {
        let profile = FlightFeelProfile::shaped(mode);
        for axis in axes(&profile) {
            let d = axis.dynamics;
            assert!(d.release_accel >= d.apply_accel, "{mode:?} release");
            assert!(d.release_jerk >= d.apply_jerk, "{mode:?} release jerk");
            assert!(d.reversal_accel >= d.apply_accel, "{mode:?} reversal floor");
            assert!(
                d.reversal_accel <= d.release_accel,
                "{mode:?} reversal ceiling"
            );
            assert!(d.reversal_jerk <= d.release_jerk, "{mode:?} reversal jerk");
        }
    }
}

#[test]
fn every_neutral_band_is_hysteretic_and_dwells() {
    // Leaving is harder than staying, or an input resting on the edge
    // chatters between commanding and not. The dwell is what keeps a
    // resting hand from commanding at all.
    for mode in SHAPED {
        let profile = FlightFeelProfile::shaped(mode);
        for axis in axes(&profile) {
            let band = axis.neutral;
            assert!(band.active_exit < band.active_enter, "{mode:?} hysteresis");
            assert!(band.dwell_ms > 0, "{mode:?} dwell");
            assert!(
                band.active_enter < axis.curve.deadzone,
                "{mode:?} band inside deadzone"
            );
        }
    }
}

#[test]
fn the_modes_differ_in_degree_and_never_in_kind() {
    // An operator who has learned one mode knows what the others will do.
    // Precision is the calmest and Agile the quickest, in every bound, on
    // every axis — never quicker in one and calmer in another.
    let ordered = SHAPED.map(FlightFeelProfile::shaped);
    for index in 0..axes(&ordered[0]).len() {
        let precision = axes(&ordered[0])[index];
        let balanced = axes(&ordered[1])[index];
        let agile = axes(&ordered[2])[index];
        assert!(precision.dynamics.apply_accel < balanced.dynamics.apply_accel);
        assert!(balanced.dynamics.apply_accel < agile.dynamics.apply_accel);
        assert!(precision.dynamics.apply_jerk < balanced.dynamics.apply_jerk);
        assert!(balanced.dynamics.apply_jerk < agile.dynamics.apply_jerk);
        // A calmer mode holds a wider quiet band and a longer dwell.
        assert!(precision.neutral.dwell_ms > balanced.neutral.dwell_ms);
        assert!(balanced.neutral.dwell_ms > agile.neutral.dwell_ms);
        assert!(precision.curve.deadzone > balanced.curve.deadzone);
        assert!(balanced.curve.deadzone > agile.curve.deadzone);
    }
}

#[test]
fn the_control_answers_on_the_first_sample() {
    // A control that does not answer immediately reads as broken however
    // smoothly it moves afterwards. At the slowest mode a full-deflection
    // demand must produce visible motion within one frame of a 50 Hz loop.
    let profile = FlightFeelProfile::shaped(FeelMode::Precision);
    let mut shaper = AxisDemandShaper::default();
    let shaped = shaper.step(1.0, 1.0, 0.02, profile.horizontal);
    assert!(
        shaped.value.abs() > 0.0,
        "the slowest mode still moves on the first sample: {}",
        shaped.value
    );
}

#[test]
fn every_shaped_mode_is_accepted_by_the_validator() {
    // A profile the validator refuses is a profile no vehicle can be given,
    // however good its numbers read.
    for mode in SHAPED {
        let profile = FlightFeelProfile::shaped(mode);
        assert!(
            ValidatedFlightFeelProfile::new(profile).is_ok(),
            "{mode:?} must be installable"
        );
    }
}

#[test]
fn the_compatibility_law_is_none_of_this() {
    // The unshaped law is kept deliberately, and this is what it costs: it
    // steps on release and on reversal, and it has no quiet band at all.
    // Nothing here should read as an accident.
    let legacy = FlightFeelProfile::shaped(FeelMode::LegacyCompatibility);
    assert_eq!(legacy.mode, FeelMode::LegacyCompatibility);
    assert!(legacy.horizontal.dynamics.release_accel > 1_000.0);
    assert_eq!(legacy.horizontal.neutral.dwell_ms, 0);
    assert_eq!(
        legacy.horizontal.neutral.active_enter,
        legacy.horizontal.neutral.active_exit
    );
}
