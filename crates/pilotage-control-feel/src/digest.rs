//! Canonical control-feel profile identity.

use core::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedFlightFeelProfile;

/// SHA-256 identity of one validated control-feel profile.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FeelDigest([u8; 32]);

impl FeelDigest {
    /// Calculate the digest from the canonical struct serialization.
    ///
    /// # Errors
    ///
    /// Returns an error if the serializer cannot encode the profile.
    pub fn calculate(profile: &ValidatedFlightFeelProfile) -> Result<Self, FeelDigestError> {
        let bytes = serde_json::to_vec(profile.profile()).map_err(FeelDigestError::Serialize)?;
        Ok(Self(Sha256::digest(bytes).into()))
    }

    /// Return the raw SHA-256 bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for FeelDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for FeelDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Control-feel digest calculation failure.
#[derive(Debug, Error)]
pub enum FeelDigestError {
    /// The canonical serializer failed.
    #[error("cannot serialize the control-feel profile")]
    Serialize(#[source] serde_json::Error),
}
