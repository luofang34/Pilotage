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

#[test]
fn the_direct_family_is_shaped_too_and_differs_between_modes() {
    // The velocity families are not the only ones a stick moves. Direct
    // flight has its own dynamics, and leaving them on the compatibility law
    // would give an operator three modes that are the same law — identical,
    // and stepping — while the control offered a choice between them.
    let modes = SHAPED.map(FlightFeelProfile::shaped);
    for (index, mode) in SHAPED.iter().enumerate() {
        let direct = modes[index].direct;
        for bound in [
            direct.tilt_rate_rps,
            direct.tilt_accel_rps2,
            direct.thrust_rate_per_s,
            direct.thrust_accel_per_s2,
        ] {
            assert!(
                bound.is_finite() && bound > 0.0,
                "{mode:?} direct bound {bound}"
            );
            assert!(bound < 1_000.0, "{mode:?} direct bound {bound} is a step");
        }
        // The brake-to-hold transition must prove stability, which a zero
        // dwell cannot: one sample under the ceiling is a moment, not a state.
        // The dwell is the whole of that proof here — no acceleration reaches
        // the detector with provenance, so the acceleration bound is left wide
        // rather than asserting a stillness nothing measures.
        assert!(modes[index].hold.stable_dwell_ms > 0, "{mode:?} hold dwell");
        assert!(
            !modes[index].hold.require_accel,
            "{mode:?} requires an acceleration no source supplies"
        );
    }

    // And they are ordered the same way the velocity families are.
    assert!(modes[0].direct.tilt_rate_rps < modes[1].direct.tilt_rate_rps);
    assert!(modes[1].direct.tilt_rate_rps < modes[2].direct.tilt_rate_rps);
    assert!(modes[0].direct.thrust_rate_per_s < modes[1].direct.thrust_rate_per_s);
    assert!(modes[1].direct.thrust_rate_per_s < modes[2].direct.thrust_rate_per_s);
    assert!(modes[0].hold.stable_dwell_ms > modes[1].hold.stable_dwell_ms);
    assert!(modes[1].hold.stable_dwell_ms > modes[2].hold.stable_dwell_ms);
}

#[test]
fn no_two_modes_are_the_same_law() {
    // A control offering three names for one law is a control that lies. This
    // compares the whole artifact rather than the axes a chosen list happens
    // to name, which is how the direct family stayed identical across all
    // three while a test called `no_axis_of_any_mode_steps` passed.
    //
    // The name and the mode are what the modes are TOLD apart by, so they are
    // normalized away before the comparison. Comparing them too would make
    // every assertion below pass on the names alone: three modes with one
    // identical set of numbers would still differ, and this test would report
    // a safety it never checked.
    let modes = SHAPED.map(FlightFeelProfile::shaped);
    let law_of = |profile: &FlightFeelProfile, named_as: &FlightFeelProfile| {
        let mut law = profile.clone();
        law.profile_id.clone_from(&named_as.profile_id);
        law.mode = named_as.mode;
        law
    };
    for left in 0..modes.len() {
        for right in (left + 1)..modes.len() {
            assert_ne!(
                modes[left],
                law_of(&modes[right], &modes[left]),
                "{:?} and {:?} are the same law under different names",
                SHAPED[left],
                SHAPED[right]
            );
        }
    }
    // And none of them is the law they replace — again on the numbers, not
    // on the name that differs by construction.
    let legacy = FlightFeelProfile::legacy_compatibility();
    for (index, mode) in SHAPED.iter().enumerate() {
        assert_ne!(
            modes[index].direct, legacy.direct,
            "{mode:?} keeps the stepping direct law"
        );
        assert_ne!(
            modes[index].hold, legacy.hold,
            "{mode:?} keeps the zero-dwell hold"
        );
    }
}

#[test]
fn every_curve_parameter_a_mode_states_actually_changes_the_curve() {
    // A parameter that reads as a difference between modes and has no effect
    // is worse than one that is absent: it documents a choice nobody made.
    // The outer exponent only applies where the outer curve blends in, so a
    // blend point of 1.0 makes every mode's outer exponent inert.
    for mode in SHAPED {
        let curve = FlightFeelProfile::shaped(mode).horizontal.curve;
        assert!(
            curve.outer_start < 1.0,
            "{mode:?} never blends its outer curve"
        );

        // Moved to a value no mode uses, so the probe is a real change even
        // where a mode's two exponents happen to be equal.
        let mut inert = curve;
        inert.outer_expo = curve.outer_expo + 0.3;
        let probe = 0.9_f32;
        assert_ne!(
            curve.apply(probe),
            inert.apply(probe),
            "{mode:?} outer exponent has no effect on the curve"
        );

        // And the centre exponent still governs near centre.
        let mut linear = curve;
        linear.center_expo = 0.0;
        let near_centre = curve.deadzone + 0.1;
        assert_ne!(
            curve.apply(near_centre),
            linear.apply(near_centre),
            "{mode:?} centre exponent has no effect near centre"
        );
    }
}
