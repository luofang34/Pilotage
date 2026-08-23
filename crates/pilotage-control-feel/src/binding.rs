//! Artifact bindings for one control-feel profile.

use core::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const DIGEST_BYTES: usize = 32;
const DIGEST_HEX_BYTES: usize = DIGEST_BYTES * 2;

/// SHA-256 identity of the device profile referenced by this artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceProfileDigest([u8; DIGEST_BYTES]);

/// SHA-256 identity of the flight-controller candidate referenced by this artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlightControllerDigest([u8; DIGEST_BYTES]);

/// Artifact identities that a control-feel profile requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBindings {
    /// Referenced device profile.
    pub device_profile_sha256: DeviceProfileDigest,
    /// Referenced flight-controller candidate.
    pub flight_controller_sha256: FlightControllerDigest,
}

impl DeviceProfileDigest {
    /// Make an identity from SHA-256 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Return the SHA-256 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }
}

impl FlightControllerDigest {
    /// Make an identity from SHA-256 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Return the SHA-256 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }
}

macro_rules! digest_wire {
    ($type:ty, $label:literal) => {
        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(self.as_bytes(), formatter)
            }
        }

        impl Serialize for $type {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let text = String::deserialize(deserializer)?;
                parse_hex(&text).map(Self::from_bytes).ok_or_else(|| {
                    serde::de::Error::custom(concat!($label, " must be 64 hex digits"))
                })
            }
        }
    };
}

digest_wire!(DeviceProfileDigest, "device_profile_sha256");
digest_wire!(FlightControllerDigest, "flight_controller_sha256");

fn write_hex(bytes: &[u8; DIGEST_BYTES], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

fn parse_hex(text: &str) -> Option<[u8; DIGEST_BYTES]> {
    if text.len() != DIGEST_HEX_BYTES || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0_u8; DIGEST_BYTES];
    for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes[index] = (high << 4) | low;
    }
    Some(bytes)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
