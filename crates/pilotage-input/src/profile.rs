//! Device profile schema v1 and sans-IO parsing (ADR-0007).
//!
//! A profile is data, not code (ADR-0007's consequence section): it is
//! parsed from JSON bytes or a `&str`, never fetched over the network by
//! this crate. Callers own retrieval; this module only validates and
//! deserializes.

mod axis;
mod button;
mod device;
mod error;

pub use axis::{AxisCalibration, AxisConfig};
pub use button::ButtonConfig;
pub use device::{DeviceIdentity, DeviceInfo, DeviceProfile, SCHEMA_VERSION};
pub use error::{EntryKind, ProfileError};

use crate::logical::{axis_id_for_name, button_id_for_name};
use std::collections::HashSet;

/// Parses a device profile from a UTF-8 JSON `&str`.
///
/// Validates `schema_version` and that every `axes[].logical` /
/// `buttons[].logical` name resolves in the well-known table
/// (`crate::logical`), so a profile with a typo fails to load rather than
/// silently dropping an axis.
///
/// # Errors
///
/// Returns [`ProfileError::MalformedJson`] if the JSON does not match the
/// schema, [`ProfileError::UnsupportedSchemaVersion`] if `schema_version`
/// is not [`SCHEMA_VERSION`], or [`ProfileError::UnknownAxisName`] /
/// [`ProfileError::UnknownButtonName`] if a logical name is unrecognized.
pub fn parse_profile_str(json: &str) -> Result<DeviceProfile, ProfileError> {
    let profile: DeviceProfile =
        serde_json::from_str(json).map_err(|source| ProfileError::MalformedJson {
            message: source.to_string(),
        })?;
    validate_profile(&profile)?;
    Ok(profile)
}

/// Parses a device profile from raw bytes, expected to be UTF-8 JSON.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidUtf8`] if `bytes` is not valid UTF-8, and
/// the same errors as [`parse_profile_str`] otherwise.
pub fn parse_profile_bytes(bytes: &[u8]) -> Result<DeviceProfile, ProfileError> {
    let text = core::str::from_utf8(bytes).map_err(|source| ProfileError::InvalidUtf8 {
        source: error::Utf8ErrorEq(source),
    })?;
    parse_profile_str(text)
}

/// Checks `schema_version` and every logical name against the well-known
/// table, without allocating a normalized copy of the profile.
fn validate_profile(profile: &DeviceProfile) -> Result<(), ProfileError> {
    if profile.schema_version != SCHEMA_VERSION {
        return Err(ProfileError::UnsupportedSchemaVersion {
            found: profile.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    let mut axis_sources = HashSet::new();
    let mut axis_names = HashSet::new();
    for axis in &profile.axes {
        axis_id_for_name(&axis.logical)?;
        if !axis_sources.insert(axis.source_index) {
            return Err(ProfileError::DuplicateSourceIndex {
                kind: EntryKind::Axis,
                source_index: axis.source_index,
            });
        }
        if !axis_names.insert(axis.logical.as_str()) {
            return Err(ProfileError::DuplicateLogicalName {
                kind: EntryKind::Axis,
                name: axis.logical.clone(),
            });
        }
        let calibration = &axis.calibration;
        let ordered = calibration.min < calibration.center && calibration.center < calibration.max;
        if !ordered {
            return Err(ProfileError::DegenerateCalibration {
                source_index: axis.source_index,
                min: calibration.min,
                center: calibration.center,
                max: calibration.max,
            });
        }
    }
    let mut button_sources = HashSet::new();
    let mut button_names = HashSet::new();
    for button in &profile.buttons {
        button_id_for_name(&button.logical)?;
        if !button_sources.insert(button.source_index) {
            return Err(ProfileError::DuplicateSourceIndex {
                kind: EntryKind::Button,
                source_index: usize::from(button.source_index),
            });
        }
        if !button_names.insert(button.logical.as_str()) {
            return Err(ProfileError::DuplicateLogicalName {
                kind: EntryKind::Button,
                name: button.logical.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ProfileError, parse_profile_bytes, parse_profile_str};

    const GOLDEN: &str = r#"{
        "schema_version": 1,
        "revision": 1,
        "device": { "vendor_id": 1, "product_id": 2, "product": "Test" },
        "description": "test profile",
        "axes": [
            {
                "source_index": 0,
                "logical": "roll",
                "invert": false,
                "deadzone": 0.05,
                "expo": 0.0,
                "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 }
            }
        ],
        "buttons": [
            { "source_index": 0, "logical": "button0" }
        ]
    }"#;

    #[test]
    fn parses_golden_profile() {
        let profile = parse_profile_str(GOLDEN).expect("parse golden");
        assert_eq!(profile.schema_version, 1);
        assert_eq!(profile.axes.len(), 1);
        assert_eq!(profile.buttons.len(), 1);
    }

    #[test]
    fn parses_from_bytes() {
        let profile = parse_profile_bytes(GOLDEN.as_bytes()).expect("parse golden bytes");
        assert_eq!(profile.revision, 1);
    }

    #[test]
    fn rejects_invalid_utf8_bytes() {
        let bytes = [0xFFu8, 0xFE, 0xFD];
        let err = parse_profile_bytes(&bytes).expect_err("should fail");
        assert!(matches!(err, ProfileError::InvalidUtf8 { .. }));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let json = GOLDEN.replace("\"schema_version\": 1", "\"schema_version\": 2");
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(
            err,
            ProfileError::UnsupportedSchemaVersion {
                found: 2,
                expected: 1
            }
        ));
    }

    #[test]
    fn rejects_unknown_axis_logical_name() {
        let json = GOLDEN.replace("\"roll\"", "\"bogus\"");
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(err, ProfileError::UnknownAxisName { .. }));
    }

    #[test]
    fn rejects_unknown_button_logical_name() {
        let json = GOLDEN.replace("\"button0\"", "\"bogus\"");
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(err, ProfileError::UnknownButtonName { .. }));
    }

    #[test]
    fn rejects_degenerate_calibration_range() {
        let json = GOLDEN.replace(
            "\"calibration\": { \"min\": -1.0, \"center\": 0.0, \"max\": 1.0 }",
            "\"calibration\": { \"min\": 0.0, \"center\": 0.0, \"max\": 0.0 }",
        );
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(err, ProfileError::DegenerateCalibration { .. }));
    }

    #[test]
    fn rejects_center_outside_min_max_range() {
        let json = GOLDEN.replace(
            "\"calibration\": { \"min\": -1.0, \"center\": 0.0, \"max\": 1.0 }",
            "\"calibration\": { \"min\": 0.0, \"center\": 20.0, \"max\": 10.0 }",
        );
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(err, ProfileError::DegenerateCalibration { .. }));
    }

    #[test]
    fn rejects_reversed_min_max() {
        let json = GOLDEN.replace(
            "\"calibration\": { \"min\": -1.0, \"center\": 0.0, \"max\": 1.0 }",
            "\"calibration\": { \"min\": 1.0, \"center\": 0.0, \"max\": -1.0 }",
        );
        let err = parse_profile_str(&json).expect_err("should fail");
        assert!(matches!(err, ProfileError::DegenerateCalibration { .. }));
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_profile_str("{ not json").expect_err("should fail");
        assert!(matches!(err, ProfileError::MalformedJson { .. }));
    }

    fn two_axis_profile(second_source: usize, second_logical: &str) -> String {
        GOLDEN.replace(
            r#""axes": ["#,
            &format!(
                r#""axes": [
            {{
                "source_index": {second_source},
                "logical": "{second_logical}",
                "invert": false,
                "deadzone": 0.05,
                "expo": 0.0,
                "calibration": {{ "min": -1.0, "center": 0.0, "max": 1.0 }}
            }},"#
            ),
        )
    }

    #[test]
    fn rejects_duplicate_axis_source_index() {
        let err = parse_profile_str(&two_axis_profile(0, "pitch")).expect_err("should fail");
        assert!(matches!(
            err,
            ProfileError::DuplicateSourceIndex {
                kind: super::EntryKind::Axis,
                source_index: 0
            }
        ));
    }

    #[test]
    fn rejects_duplicate_axis_logical_name() {
        let err = parse_profile_str(&two_axis_profile(1, "roll")).expect_err("should fail");
        assert!(matches!(
            err,
            ProfileError::DuplicateLogicalName {
                kind: super::EntryKind::Axis,
                ..
            }
        ));
    }

    #[test]
    fn rejects_duplicate_button_entries() {
        let dup_source = GOLDEN.replace(
            r#"{ "source_index": 0, "logical": "button0" }"#,
            r#"{ "source_index": 0, "logical": "button1" },
            { "source_index": 0, "logical": "button0" }"#,
        );
        let err = parse_profile_str(&dup_source).expect_err("should fail");
        assert!(matches!(
            err,
            ProfileError::DuplicateSourceIndex {
                kind: super::EntryKind::Button,
                source_index: 0
            }
        ));

        let dup_name = GOLDEN.replace(
            r#"{ "source_index": 0, "logical": "button0" }"#,
            r#"{ "source_index": 0, "logical": "button0" },
            { "source_index": 1, "logical": "button0" }"#,
        );
        let err = parse_profile_str(&dup_name).expect_err("should fail");
        assert!(matches!(
            err,
            ProfileError::DuplicateLogicalName {
                kind: super::EntryKind::Button,
                ..
            }
        ));
    }
}
