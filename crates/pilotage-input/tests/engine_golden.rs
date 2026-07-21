//! Engine-layer golden vectors: fixed raw sample + profile → exact normalized
//! axes and button edges (INPUT-01, ADR-0007).
//!
//! These pin the normalization pipeline and edge detector so a change to either
//! is a deliberate, visible edit — not a silent drift. They are the SHARED
//! reference the browser WASM engine must reproduce bit-for-bit: the same
//! vectors are asserted against `pilotage-input` here and against the compiled
//! wasm in `clients/web/control-runtime.test.mjs`, so native and browser output
//! can never diverge unnoticed.
//!
//! Every vector lands on `{-1, 0, 1}` (or inside a deadzone, so exactly `0`) to
//! stay independent of `powf` last-bit differences across platform libms.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_input::{
    AxisCalibration, AxisConfig, ButtonTracker, DeviceIdentity, DeviceInfo, DeviceProfile,
    content_digest, normalize_axis, select_by_identity,
};
use pilotage_protocol::ButtonEdge;

/// A `[-1, 0, 1]` calibration: raw units already lie in `[-1, 1]`.
fn unit_calibration() -> AxisCalibration {
    AxisCalibration {
        min: -1.0,
        center: 0.0,
        max: 1.0,
    }
}

fn axis(source_index: usize, logical: &str, invert: bool, deadzone: f32) -> AxisConfig {
    AxisConfig {
        source_index,
        logical: logical.to_owned(),
        invert,
        deadzone,
        expo: 0.0,
        calibration: unit_calibration(),
    }
}

/// The golden profile: roll (plain), pitch (deadzone 0.1), yaw (inverted).
fn golden_profile() -> DeviceProfile {
    DeviceProfile {
        schema_version: 1,
        revision: 7,
        device: DeviceInfo {
            vendor_id: 0x1209,
            product_id: 0x4f54,
            product: Some("golden".to_owned()),
        },
        description: None,
        axes: vec![
            axis(0, "roll", false, 0.0),
            axis(1, "pitch", false, 0.1),
            axis(2, "yaw", true, 0.0),
        ],
        buttons: vec![],
    }
}

#[test]
fn normalized_axes_match_the_golden_vectors() {
    let profile = golden_profile();
    // (axis source_index, raw input, expected normalized value).
    let vectors: [(usize, f32, f32); 9] = [
        (0, -1.0, -1.0),
        (0, 0.0, 0.0),
        (0, 1.0, 1.0),
        (1, 0.05, 0.0), // inside the 0.1 deadzone → exactly zero
        (1, 1.0, 1.0),
        (1, -1.0, -1.0),
        (2, 1.0, -1.0), // inverted
        (2, -1.0, 1.0),
        (2, 0.0, 0.0),
    ];
    for (source_index, raw, expected) in vectors {
        let config = &profile.axes[source_index];
        let normalized = normalize_axis(raw, config);
        assert!(!normalized.fault, "axis {source_index} raw {raw} faulted");
        assert_eq!(
            normalized.value, expected,
            "axis {source_index} raw {raw} → {} (want {expected})",
            normalized.value
        );
    }
}

#[test]
fn a_non_finite_axis_reads_zero_and_faults() {
    let profile = golden_profile();
    let normalized = normalize_axis(f32::NAN, &profile.axes[0]);
    assert_eq!(normalized.value, 0.0);
    assert!(normalized.fault, "a non-finite raw must fault");
}

#[test]
fn button_edges_match_the_golden_sequence() {
    let mut tracker = ButtonTracker::new();
    assert_eq!(tracker.update(0b0000_0001), [(0, ButtonEdge::Pressed)]);
    assert_eq!(tracker.update(0b0000_0011), [(1, ButtonEdge::Pressed)]);
    assert_eq!(
        tracker.update(0b0000_0000),
        [(0, ButtonEdge::Released), (1, ButtonEdge::Released)],
        "releases report in ascending source-index order"
    );
}

#[test]
fn a_held_button_fires_exactly_one_pressed_edge() {
    let mut tracker = ButtonTracker::new();
    assert_eq!(tracker.update(0b0000_0001), [(0, ButtonEdge::Pressed)]);
    assert!(
        tracker.update(0b0000_0001).is_empty(),
        "a still-held button fires no new edge"
    );
}

#[test]
fn device_selection_prefers_the_exact_profile_then_the_generic_fallback() {
    let vendor = golden_profile();
    let mut generic = golden_profile();
    generic.device.vendor_id = 0;
    generic.device.product_id = 0;
    let candidates = [generic, vendor];

    let exact = select_by_identity(
        DeviceIdentity {
            vendor_id: 0x1209,
            product_id: 0x4f54,
        },
        &candidates,
    )
    .expect("the exact device resolves");
    assert_eq!(exact.identity().vendor_id, 0x1209);

    let fallback = select_by_identity(
        DeviceIdentity {
            vendor_id: 0xdead,
            product_id: 0xbeef,
        },
        &candidates,
    )
    .expect("an unknown device falls back to the generic profile");
    assert_eq!(fallback.identity(), DeviceIdentity::WILDCARD);
}

#[test]
fn the_content_digest_binds_exact_bytes() {
    let bytes = br#"{"schema_version":1,"revision":7}"#;
    assert_eq!(content_digest(bytes), content_digest(bytes), "stable");
    let mut altered = bytes.to_vec();
    altered[0] ^= 0x01;
    assert_ne!(
        content_digest(bytes),
        content_digest(&altered),
        "a single-byte change changes the digest"
    );
}
