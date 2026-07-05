//! Button configuration schema (ADR-0007 schema v1).

use serde::{Deserialize, Serialize};

/// Configuration for a single physical button, as declared in a device
/// profile's `buttons` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ButtonConfig {
    /// Index of this button in `RawDeviceSample::buttons` (bit position).
    pub source_index: u8,
    /// Well-known logical button name (resolved via `crate::logical`).
    pub logical: String,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::ButtonConfig;

    #[test]
    fn button_config_roundtrips_through_json() {
        let config = ButtonConfig {
            source_index: 3,
            logical: "button3".to_string(),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ButtonConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, config);
    }
}
