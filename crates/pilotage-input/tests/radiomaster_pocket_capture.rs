//! Integration test: replays the RadioMaster Pocket capture fixture through
//! the `pilotage-input` normalization pipeline (ADR-0007) using the
//! `radiomaster-pocket.json` profile, and checks the pipeline never
//! produces a non-finite or out-of-range axis value.
//!
//! The raw-report-to-[`RawDeviceSample`] decoder below is intentionally
//! duplicated from `tools/hid-probe` rather than imported: this crate's
//! tests exercise only the public API of `pilotage-input` plus fixture
//! data, never a `tools/` binary (ADR-0002 keeps the sans-IO core
//! independent of native platform tooling).

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_input::{DeviceProfile, RawDeviceSample, normalize_axis, parse_profile_str};
use pilotage_timing::MonoTimestamp;
use serde::Deserialize;

const PROFILE_JSON: &str = include_str!("../registry/radiomaster-pocket.json");
const CAPTURE_JSON: &str = include_str!("../registry/fixtures/radiomaster-pocket-capture.json");

/// Number of packed button bytes preceding the axis fields in a RadioMaster
/// Pocket input report, per the HID report descriptor verified with
/// `tools/hid-probe` (24 buttons at 1 bit each = 3 bytes).
const BUTTON_BYTE_COUNT: usize = 3;

/// One row of the capture fixture's `reports` array.
#[derive(Debug, Deserialize)]
struct CapturedReport {
    /// Milliseconds since capture start; carried through as the sample's
    /// timestamp so ordering is preserved even though tests do not assert
    /// on absolute timing.
    t_ms: u64,
    /// Raw report bytes as lowercase hex pairs separated by spaces.
    bytes_hex: String,
}

/// Top-level shape of the capture fixture JSON.
#[derive(Debug, Deserialize)]
struct Capture {
    /// Captured reports in capture order.
    reports: Vec<CapturedReport>,
}

/// Parses a `"01 ff 00"`-style hex string back into raw bytes.
fn parse_hex(hex: &str) -> Vec<u8> {
    hex.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("valid hex byte in fixture"))
        .collect()
}

/// Decodes one raw HID input report into a [`RawDeviceSample`]: the first
/// [`BUTTON_BYTE_COUNT`] bytes are a little-endian button bitmap, the rest
/// are little-endian `u16` axis words, exactly matching the on-wire layout
/// `tools/hid-probe` observed for this device (see
/// `registry/radiomaster-pocket.json`'s description field).
fn decode_report(bytes: &[u8], t_ms: u64) -> RawDeviceSample {
    let button_bytes = &bytes[..BUTTON_BYTE_COUNT];
    let mut buttons: u64 = 0;
    for (byte_index, byte) in button_bytes.iter().enumerate() {
        buttons |= u64::from(*byte) << (8 * byte_index);
    }
    let axes: Vec<f32> = bytes[BUTTON_BYTE_COUNT..]
        .chunks_exact(2)
        .map(|pair| f32::from(u16::from_le_bytes([pair[0], pair[1]])))
        .collect();
    RawDeviceSample::new(axes, buttons, MonoTimestamp::from_nanos(t_ms * 1_000_000))
}

/// Loads the built-in RadioMaster Pocket profile from the registry fixture.
fn load_profile() -> DeviceProfile {
    parse_profile_str(PROFILE_JSON).expect("radiomaster-pocket.json parses")
}

/// Loads and decodes the capture fixture into raw samples.
fn load_capture() -> Vec<RawDeviceSample> {
    let capture: Capture = serde_json::from_str(CAPTURE_JSON).expect("capture fixture parses");
    capture
        .reports
        .iter()
        .map(|report| decode_report(&parse_hex(&report.bytes_hex), report.t_ms))
        .collect()
}

#[test]
fn every_captured_report_normalizes_to_finite_unit_range_axes() {
    let profile = load_profile();
    let samples = load_capture();
    assert!(!samples.is_empty(), "fixture must contain captured reports");

    for sample in &samples {
        for axis_config in &profile.axes {
            let raw = sample.axes[axis_config.source_index];
            let normalized = normalize_axis(raw, axis_config);
            assert!(
                !normalized.fault,
                "axis {} faulted on raw value {raw}",
                axis_config.logical
            );
            assert!(
                normalized.value.is_finite(),
                "axis {} produced non-finite value from raw {raw}",
                axis_config.logical
            );
            assert!(
                (-1.0..=1.0).contains(&normalized.value),
                "axis {} value {} out of [-1, 1] range",
                axis_config.logical,
                normalized.value
            );
        }
    }
}

#[test]
fn self_centering_axes_decode_near_zero_at_rest() {
    let profile = load_profile();
    let samples = load_capture();
    let self_centering = ["roll", "pitch", "yaw"];

    for sample in &samples {
        for axis_config in &profile.axes {
            if !self_centering.contains(&axis_config.logical.as_str()) {
                continue;
            }
            let raw = sample.axes[axis_config.source_index];
            let normalized = normalize_axis(raw, axis_config);
            assert!(
                normalized.value.abs() < 0.05,
                "resting axis {} expected near zero, got {} from raw {raw}",
                axis_config.logical,
                normalized.value
            );
        }
    }
}

#[test]
fn non_centering_axes_decode_near_zero_at_idle() {
    let profile = load_profile();
    let samples = load_capture();
    let non_centering = ["throttle", "aux1", "aux2", "aux3", "aux4"];

    for sample in &samples {
        for axis_config in &profile.axes {
            if !non_centering.contains(&axis_config.logical.as_str()) {
                continue;
            }
            let raw = sample.axes[axis_config.source_index];
            let normalized = normalize_axis(raw, axis_config);
            assert!(
                normalized.value.abs() < 0.05,
                "idle axis {} expected near zero, got {} from raw {raw}",
                axis_config.logical,
                normalized.value
            );
        }
    }
}

#[test]
fn capture_fixture_reports_have_the_expected_byte_length() {
    let samples = load_capture();
    let profile = load_profile();
    let expected_axis_count = profile.axes.len();
    for sample in &samples {
        assert_eq!(
            sample.axes.len(),
            expected_axis_count,
            "decoded axis count must match the profile's axis count"
        );
    }
}
