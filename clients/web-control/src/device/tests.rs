//! Device-stage unit tests: embedded-set integrity, identity parsing, shared
//! selection (including the fail-closed ambiguity path), and translation.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_input::{DeviceIdentity, parse_profile_bytes};

use super::{
    CompiledDevice, DUALSENSE_JSON, DeviceStage, GENERIC_PAD_JSON, KEYBOARD_JSON,
    RADIOMASTER_POCKET_JSON, SelectOutcome, parse_gamepad_identity,
};
use crate::sample::{ButtonSample, RawSample};

/// Every embedded profile must parse AND compile to a device map — a broken
/// built-in would otherwise fail closed at runtime with no visible cause.
#[test]
fn the_embedded_device_set_compiles() {
    for bytes in [
        KEYBOARD_JSON,
        GENERIC_PAD_JSON,
        DUALSENSE_JSON,
        RADIOMASTER_POCKET_JSON,
    ] {
        let profile = parse_profile_bytes(bytes).expect("embedded profile parses");
        CompiledDevice::from_profile(&profile).expect("embedded profile compiles");
    }
    let stage = DeviceStage::new();
    assert!(stage.keyboard.is_some(), "keyboard map present");
    assert_eq!(
        stage.layers.len(),
        3,
        "gamepad candidate set (built-in layer)"
    );
    assert!(stage.pad.is_some(), "wildcard pad map pre-selected");
}

#[test]
fn chromium_and_firefox_gamepad_ids_parse_to_the_same_identity() {
    let expected = DeviceIdentity {
        vendor_id: 0x054c,
        product_id: 0x0ce6,
    };
    assert_eq!(
        parse_gamepad_identity(
            "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)"
        ),
        expected
    );
    assert_eq!(
        parse_gamepad_identity("054c-0ce6-DualSense Wireless Controller"),
        expected
    );
}

#[test]
fn an_unparsable_gamepad_id_is_the_wildcard_identity() {
    for id in [
        "",
        "Some Pad",
        "xbox-controller",
        "Vendor: zzzz Product: 0ce6",
    ] {
        assert_eq!(parse_gamepad_identity(id), DeviceIdentity::WILDCARD, "{id}");
    }
}

#[test]
fn a_known_pad_selects_its_exact_profile() {
    let mut stage = DeviceStage::new();
    let outcome = stage
        .select_pad("DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)");
    assert_eq!(outcome, SelectOutcome::Exact);
    assert_eq!(stage.pad_label(), "Sony DualSense");
}

#[test]
fn an_unknown_pad_falls_back_to_the_generic_profile() {
    let mut stage = DeviceStage::new();
    let outcome = stage.select_pad("Mystery Pad (Vendor: dead Product: beef)");
    assert_eq!(outcome, SelectOutcome::Fallback);
    assert_eq!(stage.pad_label(), "Generic Gamepad (standard mapping)");
}

/// Two candidates claiming one identity refuse the pad outright: no map is
/// kept, so a tick from that pad reads an empty sample and drives nothing.
#[test]
fn an_ambiguous_registry_refuses_the_pad() {
    let mut stage = DeviceStage::new();
    // A duplicate claim WITHIN one layer is ambiguous; layered precedence
    // only arbitrates ACROSS layers.
    assert!(stage.add_profile(pilotage_input::ProfileLayer::BuiltIn, DUALSENSE_JSON));
    let outcome = stage
        .select_pad("DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)");
    assert_eq!(outcome, SelectOutcome::Refused);
    assert_eq!(stage.pad_label(), "");

    let mut out = RawSample::default();
    let (axes, buttons) = stage.pad_sample(&[1.0, 1.0, 1.0, 1.0], &[], &mut out);
    assert_eq!((axes, buttons), (0, 0), "a refused pad samples empty");
}

#[test]
fn the_generic_profile_routes_slots_one_to_one() {
    let stage = DeviceStage::new();
    let mut out = RawSample::default();
    let buttons = [
        ButtonSample {
            pressed: true,
            value: 1.0,
        },
        ButtonSample {
            pressed: false,
            value: 0.25,
        },
    ];
    stage.pad_sample(&[0.5, -0.5, 0.25, -1.0], &buttons, &mut out);
    assert_eq!(out.axes[0], 0.5);
    assert_eq!(out.axes[1], -0.5);
    assert_eq!(out.axes[2], 0.25);
    assert_eq!(out.axes[3], -1.0);
    assert!(out.buttons[0].pressed);
    assert_eq!(out.buttons[1].value, 0.25);
}

/// A non-finite raw axis normalizes to neutral through the shared engine —
/// the device stage inherits the engine's fault handling, not a JS guess.
#[test]
fn a_non_finite_pad_axis_reads_neutral() {
    let stage = DeviceStage::new();
    let mut out = RawSample::default();
    stage.pad_sample(&[f32::NAN, 0.0, 0.0, 0.0], &[], &mut out);
    assert_eq!(out.axes[0], 0.0);
}

/// The RadioMaster Pocket profile actually REROUTES: AETR device order lands
/// on canonical stick positions, with the inversions its data declares.
#[test]
fn the_radiomaster_profile_reroutes_aetr_to_canonical_slots() {
    let mut stage = DeviceStage::new();
    let outcome = stage.select_pad("1209-4f54-RadioMaster Pocket");
    assert_eq!(outcome, SelectOutcome::Exact);
    let mut out = RawSample::default();
    // Device order: 0 = aileron, 1 = elevator, 2 = throttle, 3 = rudder.
    stage.pad_sample(&[0.25, 0.5, 0.75, -0.5], &[], &mut out);
    assert_eq!(out.axes[0], -0.5, "slot0 (left X) <- rudder");
    assert_eq!(out.axes[1], -0.75, "slot1 (left Y) <- throttle, inverted");
    assert_eq!(out.axes[2], 0.25, "slot2 (right X) <- aileron");
    assert_eq!(out.axes[3], -0.5, "slot3 (right Y) <- elevator, inverted");
}

/// Keyboard synthesis reproduces the retired shell table bit-for-bit: the
/// same slots, the same deflections, the same axis/button counts, and the
/// same later-entry-wins rule for two held keys on one slot.
#[test]
fn keyboard_synthesis_matches_the_retired_shell_table() {
    let mut stage = DeviceStage::new();
    let mut out = RawSample::default();

    stage.key_event("w", true);
    stage.key_event("ArrowRight", true);
    stage.key_event("Enter", true);
    let (axis_count, button_count) = stage.key_sample(&mut out);
    assert_eq!((axis_count, button_count), (4, 10));
    assert_eq!(out.axes[1], -1.0, "w climbs");
    assert_eq!(out.axes[2], 1.0, "ArrowRight yaws right");
    assert!(out.buttons[9].pressed, "Enter arms");
    assert_eq!(out.buttons[9].value, 1.0);

    // s and w both held: w is the later entry on slot1 and wins.
    stage.key_event("s", true);
    stage.key_sample(&mut out);
    assert_eq!(out.axes[1], -1.0);

    stage.key_event("w", false);
    stage.key_sample(&mut out);
    assert_eq!(out.axes[1], 1.0, "s alone descends");
}

#[test]
fn clearing_held_keys_neutralizes_the_synthesized_sample() {
    let mut stage = DeviceStage::new();
    let mut out = RawSample::default();
    stage.key_event("w", true);
    stage.key_event("Backspace", true);
    stage.clear_keys();
    stage.key_sample(&mut out);
    assert!(out.axes.iter().all(|axis| *axis == 0.0));
    assert!(out.buttons.iter().all(|button| !button.pressed));
}
