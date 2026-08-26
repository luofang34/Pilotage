//! What each airframe can deliver, and which shipped laws ask for more.

#![allow(clippy::expect_used, clippy::panic)]

use super::AirframeLimits;
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
    // be one the vehicle cannot reach. Recorded here rather than quietly
    // corrected: changing a law that has been flown is a tuning decision, and
    // it belongs to a run that measures the result.
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
