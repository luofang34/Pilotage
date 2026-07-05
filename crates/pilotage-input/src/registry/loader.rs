//! Sans-IO loading of device profiles from in-memory bytes or `&str`.
//!
//! Retrieval (filesystem, network, embedded asset) is the caller's job per
//! ADR-0002; this module only turns already-in-hand bytes into a validated
//! [`DeviceProfile`], reusing `crate::profile`'s schema-version and
//! logical-name checks.

use crate::profile::{DeviceProfile, ProfileError, parse_profile_bytes, parse_profile_str};

/// Loads a device profile from a UTF-8 JSON `&str`.
///
/// # Errors
///
/// See [`crate::profile::parse_profile_str`].
pub fn load_profile_str(json: &str) -> Result<DeviceProfile, ProfileError> {
    parse_profile_str(json)
}

/// Loads a device profile from raw bytes, expected to be UTF-8 JSON.
///
/// # Errors
///
/// See [`crate::profile::parse_profile_bytes`].
pub fn load_profile_bytes(bytes: &[u8]) -> Result<DeviceProfile, ProfileError> {
    parse_profile_bytes(bytes)
}

/// The built-in generic-gamepad profile shipped with this crate
/// (`registry/generic-gamepad.json`), embedded at compile time so it is
/// always available without filesystem access at runtime.
pub const GENERIC_GAMEPAD_JSON: &str = include_str!("../../registry/generic-gamepad.json");

/// Loads the built-in generic-gamepad profile.
///
/// # Errors
///
/// Returns [`ProfileError`] only if the embedded registry file itself was
/// corrupted at build time; under normal operation this always succeeds.
pub fn load_builtin_generic_gamepad() -> Result<DeviceProfile, ProfileError> {
    load_profile_str(GENERIC_GAMEPAD_JSON)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::load_builtin_generic_gamepad;

    #[test]
    fn builtin_generic_gamepad_loads_successfully() {
        let profile = load_builtin_generic_gamepad().expect("built-in profile loads");
        assert_eq!(profile.schema_version, 1);
        assert!(!profile.axes.is_empty());
    }
}
