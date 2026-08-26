use serde::{Deserialize, Serialize};

use crate::{Digest, TuneError};

use super::digest::digest_bytes;
use super::invalid_terminal;

/// The largest saved terminal diagnostic projection.
pub const MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES: usize = 2_048;

/// A bounded projection and identity for one full diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalDiagnostic {
    projection: String,
    projection_digest: Digest,
    full_digest: Digest,
    byte_count: u64,
}

impl RunTerminalDiagnostic {
    /// Creates a bounded diagnostic identity from the full text.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the diagnostic is empty or too large to count.
    pub fn new(detail: &str) -> Result<Self, TuneError> {
        if detail.is_empty() {
            return Err(invalid_terminal("a terminal diagnostic is empty"));
        }
        let projection = bounded_projection(detail);
        let byte_count = u64::try_from(detail.len())
            .map_err(|_| invalid_terminal("a terminal diagnostic byte count is too large"))?;
        Ok(Self {
            projection_digest: digest_bytes(projection.as_bytes()),
            full_digest: digest_bytes(detail.as_bytes()),
            projection,
            byte_count,
        })
    }

    /// Validates the bounded projection and its identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one diagnostic field is inconsistent.
    pub fn validate(&self) -> Result<(), TuneError> {
        let projection_bytes = u64::try_from(self.projection.len())
            .map_err(|_| invalid_terminal("a terminal projection byte count is too large"))?;
        let maximum = u64::try_from(MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES)
            .map_err(|_| invalid_terminal("the terminal projection limit is too large"))?;
        if self.projection.is_empty()
            || self.projection.len() > MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES
            || self.projection_digest.is_zero()
            || self.full_digest.is_zero()
            || self.projection_digest != digest_bytes(self.projection.as_bytes())
        {
            return Err(invalid_terminal(
                "the terminal diagnostic projection is inconsistent",
            ));
        }
        let complete =
            self.byte_count == projection_bytes && self.full_digest == self.projection_digest;
        let truncated = self.byte_count > maximum
            && self.projection.len() >= MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES.saturating_sub(3);
        if !complete && !truncated {
            return Err(invalid_terminal(
                "the terminal diagnostic is not in a canonical bounded form",
            ));
        }
        Ok(())
    }

    /// Returns the bounded diagnostic projection.
    #[must_use]
    pub fn projection(&self) -> &str {
        &self.projection
    }

    /// Returns the digest of the full diagnostic.
    #[must_use]
    pub const fn full_digest(&self) -> Digest {
        self.full_digest
    }

    /// Returns the full diagnostic byte count.
    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }
}

fn bounded_projection(detail: &str) -> String {
    if detail.len() <= MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES {
        return detail.to_owned();
    }
    let mut end = MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES;
    while !detail.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    detail[..end].to_owned()
}
