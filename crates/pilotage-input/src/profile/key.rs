//! Key-binding schema (ADR-0007 schema v1): a keyboard-class device as
//! profile data, so no key → control table ever lives in shell code.

use serde::{Deserialize, Serialize};

/// One held key driving a logical axis to a fixed deflection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyAxisBinding {
    /// Well-known logical axis name (resolved via `crate::logical`).
    pub logical: String,
    /// The deflection a held key commands, in normalized `[-1, 1]`.
    pub value: f32,
}

/// One key of a keyboard-class device bound to a logical input.
///
/// `key` is the canonical DOM `KeyboardEvent.key` value with single letters
/// lower-cased (`"w"`, `"ArrowUp"`, `"Enter"`), so a shell passes key events
/// through verbatim and the profile stays the only mapping authority.
/// Exactly one of `axis` / `button` must be set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    /// The canonical key value this binding matches.
    pub key: String,
    /// Axis target: while the key is held, the axis reads `value`.
    #[serde(default)]
    pub axis: Option<KeyAxisBinding>,
    /// Button target: while the key is held, this logical button reads
    /// pressed (well-known button name, resolved via `crate::logical`).
    #[serde(default)]
    pub button: Option<String>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::KeyBinding;

    #[test]
    fn key_bindings_roundtrip_through_json() {
        let json = r#"{ "key": "w", "axis": { "logical": "slot1", "value": -1.0 } }"#;
        let binding: KeyBinding = serde_json::from_str(json).expect("deserialize");
        assert_eq!(binding.key, "w");
        assert_eq!(binding.axis.as_ref().expect("axis target").logical, "slot1");
        assert!(binding.button.is_none());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let json = r#"{ "key": "w", "scope": "vehicle.motion" }"#;
        assert!(serde_json::from_str::<KeyBinding>(json).is_err());
    }
}
