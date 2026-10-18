//! Device-stage unit tests: embedded-set integrity, identity parsing, shared
//! selection (including the fail-closed ambiguity path), and translation.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_input::{DeviceIdentity, parse_profile_bytes};

use super::{
    CompiledDevice, DUALSENSE_JSON, DeviceStage, GENERIC_PAD_JSON, KEYBOARD_JSON, NAMED_IDENTITIES,
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
        4,
        "built-in layer: keyboard + three gamepad profiles"
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

#[test]
fn the_radiomaster_effective_profile_has_the_configured_digest() {
    let mut stage = DeviceStage::new();
    assert_eq!(
        stage.select_pad("1209-4f54-RadioMaster Pocket"),
        SelectOutcome::Exact
    );
    assert_eq!(
        stage.pad_digest(),
        Some([
            0x32, 0x85, 0x73, 0x85, 0x65, 0x47, 0xb1, 0x64, 0x6e, 0xca, 0xe8, 0x74, 0x38, 0x15,
            0xbe, 0x16, 0x1d, 0x5a, 0xba, 0x9b, 0x97, 0x4a, 0xaa, 0xfd, 0xf9, 0x75, 0x6c, 0xe3,
            0x04, 0x6d, 0x0d, 0x17,
        ])
    );
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
    // Twelve buttons: the retired shell's ten plus the payload pair
    // (g -> button6 quasimode, h -> button11 recenter).
    assert_eq!((axis_count, button_count), (4, 12));
    assert_eq!(out.axes[1], -1.0, "w climbs");
    assert_eq!(out.axes[2], 1.0, "ArrowRight yaws right");
    assert!(out.buttons[9].pressed, "Enter arms");
    assert_eq!(out.buttons[9].value, 1.0);

    stage.key_event("g", true);
    stage.key_event("h", true);
    stage.key_sample(&mut out);
    assert!(out.buttons[6].pressed, "g holds the gimbal quasimode");
    assert!(out.buttons[11].pressed, "h presses the recenter");
    stage.key_event("g", false);
    stage.key_event("h", false);
    stage.key_sample(&mut out);
    assert!(!out.buttons[6].pressed);
    assert!(!out.buttons[11].pressed);

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

#[test]
fn named_pads_route_straight_through() {
    // Name matching exists for one caller: the Apple client, which has no USB
    // pair to offer. That client reads GameController's `leftThumbstick` and
    // `rightThumbstick`, so the axes it sends are already in canonical slot
    // order — a profile that reroutes would be correcting input that arrives
    // correct, putting the sticks on the wrong controls.
    //
    // Adding a rerouting pad to the name list is therefore a flight defect and
    // not a naming preference. This is what stops it.
    for (name, _) in NAMED_IDENTITIES {
        let mut stage = DeviceStage::new();
        assert_eq!(
            stage.select_pad(name),
            SelectOutcome::Exact,
            "{name} is listed but resolves to no profile"
        );
        for source in 0..4 {
            let mut axes = [0.0_f32; 4];
            axes[source] = 1.0;
            let mut out = RawSample::default();
            stage.pad_sample(&axes, &[], &mut out);
            assert_eq!(
                out.axes[source], 1.0,
                "{name} sends source axis {source} somewhere other than slot {source}, \
                 or inverts it; a canonical stick would arrive on the wrong control"
            );
            for (slot, value) in out.axes.iter().enumerate().take(4) {
                assert!(
                    slot == source || *value == 0.0,
                    "{name} leaks source axis {source} into slot {slot}"
                );
            }
        }
    }
}
