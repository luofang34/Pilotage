//! What each airframe can deliver, and which shipped laws ask for more.

#![allow(clippy::expect_used, clippy::panic)]

use super::{AirframeLimits, MOST_AGGRESSIVE_SHARE};
use crate::{FeelMode, FlightFeelProfile};

#[test]
fn each_airframes_ceiling_follows_from_the_tilt_its_preset_states() {
    // g*tan(theta), and nothing else. Stated so a change to either airframe's
    // tilt limit shows up here as a number rather than as a feel nobody can
    // account for.
    let x500 = AirframeLimits::X500.horizontal_accel_ceiling_mps2();
    let alia = AirframeLimits::ALIA250.horizontal_accel_ceiling_mps2();
    assert!((x500 - 3.58).abs() < 0.01, "x500 ceiling {x500}");
    assert!((alia - 4.74).abs() < 0.01, "alia250 ceiling {alia}");
    // The Alia may tilt further, so it may accelerate harder. A law shared
    // between them is bounded by the lower of the two, not the higher.
    assert!(x500 < alia, "the x500 is the tighter airframe");
}

#[test]
fn the_alia_law_asks_the_x500_for_more_than_it_has() {
    // This is why the x500 flying the Alia law is not merely a naming problem.
    // Above the ceiling the demand slews faster than the vehicle can follow,
    // so Balanced and Agile converge on the x500: the operator feels the tilt
    // limit in both, and the mode they chose stops being the thing they feel.
    let asks = |mode| {
        FlightFeelProfile::shaped(mode)
            .horizontal
            .dynamics
            .apply_accel
    };
    let x500 = AirframeLimits::X500;

    assert!(
        x500.share_of_ceiling(asks(FeelMode::Precision)) < 1.0,
        "Precision is within the x500's reach"
    );
    assert!(
        x500.share_of_ceiling(asks(FeelMode::Balanced)) > 1.0,
        "Balanced already exceeds it"
    );
    assert!(
        x500.share_of_ceiling(asks(FeelMode::Agile)) > 1.0,
        "and Agile exceeds it further"
    );
}

#[test]
fn even_the_alia_cannot_fly_its_own_agile_law() {
    // Agile asks 6.5 m/s^2 of an airframe that can produce 4.74. The mode is
    // not wrong to be the most aggressive of the three; the number is wrong to
    // be one the vehicle cannot reach.
    //
    // This is the UNSHAPED family, which names no airframe and so cannot know
    // what any of them can deliver. `shaped_for` is where the number meets a
    // vehicle, and it is the only thing entitled to move it.
    let agile = FlightFeelProfile::shaped(FeelMode::Agile)
        .horizontal
        .dynamics
        .apply_accel;
    let share = AirframeLimits::ALIA250.share_of_ceiling(agile);
    assert!(
        share > 1.0,
        "agile is inside the Alia's reach after all: {share}"
    );
    assert!(
        share < 1.5,
        "the overshoot grew past what was measured and filed: {share}"
    );
}

/// What one mode's shaped law asks of the horizontal axis.
fn shaped_ask(limits: AirframeLimits, mode: FeelMode) -> f32 {
    FlightFeelProfile::shaped_for(limits, mode)
        .horizontal
        .dynamics
        .apply_accel
}

#[test]
fn the_alia_keeps_the_two_modes_that_were_always_within_its_reach() {
    // Precision and Balanced have been flown on this airframe and were never
    // the problem — only Agile asked for more than the vehicle has. Shaping
    // that changed all three would be a tuning decision on two laws with no
    // run behind it, arriving disguised as a fix for the third.
    let alia = AirframeLimits::ALIA250;
    for mode in [FeelMode::Precision, FeelMode::Balanced] {
        let flown = FlightFeelProfile::shaped(mode);
        let shaped = FlightFeelProfile::shaped_for(alia, mode);
        assert_eq!(
            shaped.horizontal.dynamics, flown.horizontal.dynamics,
            "{mode:?} moved on an airframe that could always fly it"
        );
        assert_eq!(shaped.vertical.dynamics, flown.vertical.dynamics);
        assert_eq!(shaped.yaw.dynamics, flown.yaw.dynamics);
    }
}

#[test]
fn the_alias_agile_law_comes_down_to_what_the_airframe_has() {
    // The one mode that asked for more than the vehicle can produce, and the
    // only one this shaping is entitled to move.
    let alia = AirframeLimits::ALIA250;
    let share = alia.share_of_ceiling(shaped_ask(alia, FeelMode::Agile));
    assert!(
        (share - MOST_AGGRESSIVE_SHARE).abs() < 0.001,
        "agile did not land on the airframe's share: {share}"
    );
    assert!(
        shaped_ask(alia, FeelMode::Agile)
            < FlightFeelProfile::shaped(FeelMode::Agile)
                .horizontal
                .dynamics
                .apply_accel,
        "agile was left where it was"
    );
}

#[test]
fn the_x500_scales_because_clipping_would_leave_it_two_names_for_one_law() {
    // This airframe's ceiling falls below BOTH Balanced and Agile, so clipping
    // pins them to the same number and the mode control stops selecting
    // anything. The family scales instead. Recorded as the reason the two
    // airframes are shaped differently — it is the airframe that differs, not
    // the policy.
    let x500 = AirframeLimits::X500;
    let cap = x500.horizontal_accel_ceiling_mps2() * MOST_AGGRESSIVE_SHARE;
    let unshaped = |mode| {
        FlightFeelProfile::shaped(mode)
            .horizontal
            .dynamics
            .apply_accel
    };
    assert!(
        unshaped(FeelMode::Balanced).min(cap) >= unshaped(FeelMode::Agile).min(cap),
        "clipping keeps this airframe's modes apart after all — scaling is no longer justified"
    );
    // So Precision moves too, even though it was inside the ceiling. That is
    // the cost of the branch, and it is paid on an airframe that has no flown
    // law of its own to preserve.
    assert!(
        shaped_ask(x500, FeelMode::Precision) < unshaped(FeelMode::Precision),
        "precision was left where it was"
    );
}

#[test]
fn no_airframe_ships_two_modes_an_operator_cannot_tell_apart() {
    // Whichever branch an airframe takes, the mode control has to select three
    // different laws. This is the property both branches exist to hold.
    for limits in [AirframeLimits::X500, AirframeLimits::ALIA250] {
        let asks = [
            shaped_ask(limits, FeelMode::Precision),
            shaped_ask(limits, FeelMode::Balanced),
            shaped_ask(limits, FeelMode::Agile),
        ];
        assert!(
            asks[0] < asks[1] && asks[1] < asks[2],
            "{} modes are out of order or equal: {asks:?}",
            limits.id
        );
        // And far enough apart to be felt rather than merely unequal. The
        // separations are pinned so a later tilt change that squeezes two modes
        // together fails here instead of shipping.
        for pair in asks.windows(2) {
            let separation = (pair[1] - pair[0]) / pair[1];
            assert!(
                separation > 0.05,
                "{} ships modes {pair:?} that differ by {separation:.3} of the larger",
                limits.id
            );
        }
    }
}

#[test]
fn every_shipped_mode_stays_inside_the_airframe_that_flies_it() {
    // The whole point. Neither branch may leave a mode asking for a slew the
    // vehicle cannot produce, because above the ceiling the operator feels the
    // tilt limit rather than the mode they chose.
    for limits in [AirframeLimits::X500, AirframeLimits::ALIA250] {
        for mode in [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile] {
            let share = limits.share_of_ceiling(shaped_ask(limits, mode));
            assert!(
                share <= MOST_AGGRESSIVE_SHARE + 0.001,
                "{} {mode:?} asks {share} of the ceiling",
                limits.id
            );
        }
    }
}

#[test]
fn the_law_a_physical_vehicle_flies_is_outside_both_airframes() {
    // The Aviate adapter refuses a physical vehicle any profile that is not
    // `legacy_compatibility()` byte for byte, so this is the ONLY law real
    // hardware flies — and it asks for more than either airframe can produce.
    //
    // Shaping cannot reach it. The number is a fixed response another
    // repository validates against, so moving it here would make the profile
    // unloadable rather than tamer, and picking a replacement is a tuning
    // decision that needs a hardware run behind it.
    //
    // Recorded rather than corrected, so the fitting work below is not read as
    // covering the case it cannot reach.
    let legacy = FlightFeelProfile::legacy_compatibility()
        .horizontal
        .dynamics
        .apply_accel;
    let x500 = AirframeLimits::X500.share_of_ceiling(legacy);
    let alia = AirframeLimits::ALIA250.share_of_ceiling(legacy);
    assert!(
        x500 > 1.0 && alia > 1.0,
        "the legacy law came inside an airframe: x500 {x500}, alia {alia}"
    );
    assert!(
        (x500 - 1.40).abs() < 0.01,
        "the x500 overshoot moved from what was measured: {x500}"
    );
    assert!(
        (alia - 1.06).abs() < 0.01,
        "the alia overshoot moved from what was measured: {alia}"
    );
}

#[test]
fn shaping_leaves_the_legacy_law_exactly_as_the_adapter_demands_it() {
    // Not because the law fits — it does not — but because the adapter
    // compares it field by field against its own copy and refuses a mismatch.
    // A profile shaped to fit the airframe would simply fail to load.
    for limits in [AirframeLimits::X500, AirframeLimits::ALIA250] {
        let shaped = FlightFeelProfile::shaped_for(limits, FeelMode::LegacyCompatibility);
        assert_eq!(
            shaped,
            FlightFeelProfile::legacy_compatibility(),
            "{} shaped the one law that has to stay fixed",
            limits.id
        );
    }
}
