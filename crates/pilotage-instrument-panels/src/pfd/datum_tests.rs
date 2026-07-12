#![allow(clippy::expect_used, clippy::panic)]

//! ALT-01 rendering proofs: the tape identifies its reference, a
//! selection renders only against a compatible reference, and the
//! setting readout states whether it is applied.

use std::string::String;

use pilotage_instrument_state::{AltitudeClass, PanelData, Sig, SignalStatus};

use super::tests::{PfdConfig, flying, render, texts};

fn rendered(data: &PanelData) -> std::vec::Vec<String> {
    texts(&render(data, &PfdConfig::default()))
}

#[test]
fn local_relative_tape_is_labelled_rel() {
    let all = rendered(&flying());
    assert!(all.contains(&String::from("REL")), "{all:?}");
}

#[test]
fn setting_shows_as_not_applied_outside_a_barometric_reference() {
    let mut data = flying();
    data.baro_hpa = Sig::with_status(1013.0, SignalStatus::Valid);
    let all = rendered(&data);
    assert!(
        all.contains(&String::from("SET 1013")),
        "REL tape must mark the setting not applied: {all:?}"
    );
    assert!(!all.contains(&String::from("1013")), "{all:?}");
}

#[test]
fn barometric_tape_shows_the_applied_setting_plainly() {
    let mut data = flying();
    data.altitude.class = AltitudeClass::BaroIndicated;
    data.baro_hpa = Sig::with_status(1013.0, SignalStatus::Valid);
    let all = rendered(&data);
    assert!(all.contains(&String::from("1013")), "{all:?}");
    assert!(all.contains(&String::from("BARO")), "{all:?}");
}

#[test]
fn pressure_tape_shows_std() {
    let mut data = flying();
    data.altitude.class = AltitudeClass::Pressure;
    let all = rendered(&data);
    assert!(all.contains(&String::from("STD")), "{all:?}");
}

#[test]
fn incompatible_selection_never_renders_a_plausible_number() {
    let mut data = flying();
    data.selections.altitude_sel_m = Some(914.4);
    data.altitude.bug_compatible = true;
    let with_bug = rendered(&data);
    assert!(with_bug.contains(&String::from("3000")), "{with_bug:?}");

    data.altitude.bug_compatible = false;
    let without = rendered(&data);
    assert!(!without.contains(&String::from("3000")), "{without:?}");
    assert!(without.contains(&String::from("SEL REF")), "{without:?}");
}

#[test]
fn setting_mismatch_raises_the_amber_flag() {
    let mut data = flying();
    data.altitude.class = AltitudeClass::BaroIndicated;
    data.altitude.setting_mismatch = true;
    let all = rendered(&data);
    assert!(all.contains(&String::from("BARO SEL")), "{all:?}");
}

#[test]
fn unknown_reference_is_red_labelled_and_the_tape_fails() {
    let mut data = flying();
    data.altitude.class = AltitudeClass::Unknown;
    data.altitude.value_ft = Sig::with_status(0.0, SignalStatus::Failed);
    let all = rendered(&data);
    assert!(all.contains(&String::from("REF")), "{all:?}");
    assert!(
        all.contains(&String::from("ALT")),
        "failed altitude keeps its unmistakable flag: {all:?}"
    );
}
