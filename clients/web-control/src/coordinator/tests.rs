#![allow(clippy::expect_used, clippy::panic)]

//! The transactional device-swap discipline (INPUT-01): a selection that
//! changes the effective mapping cycles authority through the runtime's
//! neutral handover and installs only at the boundary, so a deflected input
//! on the new pad can never drive the old lease.

use super::ControlCoordinator;
use pilotage_input::ProfileLayer;

use crate::DEFAULT_PROFILE_BYTES;
use crate::plan::AXIS_THROTTLE;
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

const DUALSENSE_ID: &str =
    "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)";

fn pad_sample(coordinator: &ControlCoordinator, axes: &[f32], pressed: &[usize]) -> RawSample {
    let buttons: Vec<ButtonSample> = (0..16)
        .map(|i| ButtonSample {
            pressed: pressed.contains(&i),
            value: if pressed.contains(&i) { 1.0 } else { 0.0 },
        })
        .collect();
    let mut out = RawSample::default();
    coordinator.pad_sample(axes, &buttons, &mut out);
    out
}

fn session(motion_granted: bool, motion_recovered: bool) -> SessionState {
    SessionState {
        generation: 1,
        now_ms: 100_000.0,
        mode: Mode::QuadPilot,
        connected: true,
        lease_granted: false,
        lease_denied: false,
        motion_granted,
        motion_denied: false,
        motion_recovered,
    }
}

fn with_scheme() -> ControlCoordinator {
    let mut coordinator = ControlCoordinator::new();
    assert_eq!(coordinator.activate_scheme(DEFAULT_PROFILE_BYTES), 1);
    coordinator
}

fn throttle(plan: &crate::plan::ControlPlan) -> Option<f32> {
    plan.motion.as_ref().map(|frame| {
        frame
            .axes()
            .iter()
            .find(|(axis, _)| *axis == AXIS_THROTTLE)
            .map_or(f32::NAN, |(_, value)| *value)
    })
}

#[test]
fn a_device_change_swaps_transactionally_and_gates_the_deflection() {
    let mut coordinator = with_scheme();
    // Live under the generic wildcard map.
    let live = pad_sample(&coordinator, &[0.0, -1.0, 0.0, 0.0], &[]);
    let plan = coordinator.evaluate(&live, &session(true, true));
    assert_eq!(throttle(&plan), Some(1.0), "live before the swap");

    // The pad identity changes (a DualSense replaces the generic map). The
    // effective mapping changes, so the swap opens a handover: this tick is
    // FORCED NEUTRAL even though the operator still deflects full throttle.
    coordinator.select_device(DUALSENSE_ID);
    let deflected = pad_sample(&coordinator, &[0.0, -1.0, 0.0, 0.0], &[]);
    let plan = coordinator.evaluate(&deflected, &session(true, true));
    assert_eq!(throttle(&plan), Some(0.0), "the handover emits neutral");
    assert!(
        plan.motion_lease.is_some(),
        "the handover cycles the motion lease"
    );
    assert_eq!(
        coordinator.activation_revision(),
        1,
        "no install while the operator is deflected"
    );

    // Only a genuine neutral completes the handover; the new map installs
    // and the activation revision advances, while motion output is still
    // gated behind the lease reacquisition.
    let neutral = pad_sample(&coordinator, &[0.0; 4], &[]);
    coordinator.evaluate(&neutral, &session(true, true));
    assert_eq!(coordinator.activation_revision(), 2, "install advances");
    assert_eq!(coordinator.device_label(), "Sony DualSense");
    let deflected = pad_sample(&coordinator, &[0.0, -1.0, 0.0, 0.0], &[]);
    let plan = coordinator.evaluate(&deflected, &session(false, false));
    assert_eq!(
        throttle(&plan),
        None,
        "the new map cannot drive until the lease is regranted"
    );
}

#[test]
fn an_unchanged_selection_does_not_cycle_authority() {
    let mut coordinator = with_scheme();
    // The FIRST selection is a source switch (keyboard → pad): a
    // transaction. Complete it, then re-select the same identity.
    coordinator.select_device("");
    let neutral = pad_sample(&coordinator, &[0.0; 4], &[]);
    coordinator.evaluate(&neutral, &session(true, true));
    let before = coordinator.activation_revision();
    assert_eq!(before, 2, "the source switch installed");
    // Same identity, same source, same effective mapping: reconnecting the
    // pad must not fence flight.
    coordinator.select_device("");
    let plan = coordinator.evaluate(&neutral, &session(true, true));
    assert!(plan.motion_lease.is_none(), "no lease cycle on a no-op");
    assert_eq!(coordinator.activation_revision(), before);
}

#[test]
fn the_boot_source_is_the_keyboard_and_a_disconnect_returns_to_it() {
    let mut coordinator = with_scheme();
    // Before any pad selection the announcement names the KEYBOARD — never
    // a pad profile the operator is not driving with.
    assert_eq!(coordinator.device_label(), "Keyboard");
    assert!(coordinator.device_digest().is_some());
    let keyboard_digest = coordinator.device_digest();

    coordinator.select_device(DUALSENSE_ID);
    let neutral = pad_sample(&coordinator, &[0.0; 4], &[]);
    coordinator.evaluate(&neutral, &session(true, true));
    assert_eq!(coordinator.device_label(), "Sony DualSense");
    assert_ne!(coordinator.device_digest(), keyboard_digest);

    // Disconnect: control returns to the keyboard through the SAME
    // transactional path — revision advances, identity flips back.
    coordinator.deselect_device();
    assert_eq!(
        coordinator.device_label(),
        "Sony DualSense",
        "the swap is pending, not instant"
    );
    coordinator.evaluate(&neutral, &session(true, true));
    assert_eq!(coordinator.activation_revision(), 3);
    assert_eq!(coordinator.device_label(), "Keyboard");
    assert_eq!(coordinator.device_digest(), keyboard_digest);
}

#[test]
fn a_layer_override_takes_the_transactional_path() {
    let mut coordinator = with_scheme();
    // Drive with the (generic) pad first, so the pad is the active source.
    coordinator.select_device("");
    let neutral = pad_sample(&coordinator, &[0.0; 4], &[]);
    coordinator.evaluate(&neutral, &session(true, true));
    assert_eq!(coordinator.activation_revision(), 2);
    let keyboard_digest_before = {
        let stage = coordinator.stage();
        stage.keyboard_digest()
    };

    // A session-layer override for the wildcard PAD identity: swaps
    // throttle onto slot 3 instead of slot 2.
    let override_json = br#"{
      "schema_version": 1,
      "revision": 5,
      "device": { "vendor_id": 0, "product_id": 0, "product": "Session Override" },
      "axes": [
        { "source_index": 3, "logical": "slot2", "invert": false, "deadzone": 0.0, "expo": 0.0,
          "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 } }
      ],
      "buttons": [],
      "keys": []
    }"#;
    assert!(coordinator.add_device_profile(ProfileLayer::Session, override_json));
    assert_eq!(
        coordinator.activation_revision(),
        2,
        "the override is pending its handover, not live"
    );
    coordinator.evaluate(&neutral, &session(true, true));
    assert_eq!(coordinator.activation_revision(), 3);
    assert_eq!(coordinator.device_label(), "Session Override");
    assert_eq!(coordinator.device_revision(), 5);
    // The merged map now routes physical axis 3 to canonical slot 2 — and
    // the WILDCARD pad override never bled into the keyboard's bindings.
    let swapped = pad_sample(&coordinator, &[0.0, 0.0, 0.0, 1.0], &[]);
    assert_eq!(swapped.axes.get(2).copied(), Some(1.0));
    assert_eq!(
        coordinator.stage().keyboard_digest(),
        keyboard_digest_before
    );
}

#[test]
fn rejected_override_bytes_change_nothing() {
    let mut coordinator = with_scheme();
    let digest = coordinator.device_digest();
    assert!(!coordinator.add_device_profile(ProfileLayer::User, b"not json"));
    assert_eq!(coordinator.device_digest(), digest);
    assert_eq!(coordinator.activation_revision(), 1);
}
