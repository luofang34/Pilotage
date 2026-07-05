//! Device identity and top-level profile schema (ADR-0007 schema v1).

use serde::{Deserialize, Serialize};

use super::axis::AxisConfig;
use super::button::ButtonConfig;

/// The schema version this crate parses. Profiles declaring any other
/// `schema_version` are rejected with a typed error at load time.
pub const SCHEMA_VERSION: u32 = 1;

/// USB vendor/product identity of the physical device a profile targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// USB vendor ID.
    pub vendor_id: u16,
    /// USB product ID.
    pub product_id: u16,
}

/// Device identity plus an optional human-readable product name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// USB vendor ID.
    pub vendor_id: u16,
    /// USB product ID.
    pub product_id: u16,
    /// Human-readable product name, if known.
    pub product: Option<String>,
}

/// A versioned device profile: schema-v1 JSON mapping a physical device's
/// axes and buttons onto the canonical logical input model (ADR-0007).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    /// Schema version this profile was authored against. Must equal
    /// [`SCHEMA_VERSION`] to load successfully.
    pub schema_version: u32,
    /// Monotonically increasing revision of this profile's content, carried
    /// on outgoing control frames (ADR-0009) so a receiver can tell which
    /// calibration produced a given frame.
    pub revision: u32,
    /// Identity of the physical device this profile targets.
    pub device: DeviceInfo,
    /// Optional free-text description for humans browsing the registry.
    pub description: Option<String>,
    /// Per-axis configuration.
    pub axes: Vec<AxisConfig>,
    /// Per-button configuration.
    pub buttons: Vec<ButtonConfig>,
}

impl DeviceProfile {
    /// Returns the device identity (vendor/product ID pair) this profile
    /// targets, discarding the optional product name.
    #[must_use]
    pub const fn identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            vendor_id: self.device.vendor_id,
            product_id: self.device.product_id,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DeviceIdentity, DeviceInfo, DeviceProfile};

    #[test]
    fn identity_extracts_vendor_and_product() {
        let profile = DeviceProfile {
            schema_version: 1,
            revision: 1,
            device: DeviceInfo {
                vendor_id: 0x1234,
                product_id: 0x5678,
                product: Some("Test Device".to_string()),
            },
            description: None,
            axes: vec![],
            buttons: vec![],
        };
        assert_eq!(
            profile.identity(),
            DeviceIdentity {
                vendor_id: 0x1234,
                product_id: 0x5678,
            }
        );
    }
}
