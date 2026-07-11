//! Source-attachment identity providers.

use pilotage_adapter_api::SourceIncarnation;

use crate::error::AviateAdapterError;

/// Supplies a non-repeating identity for one adapter attachment.
///
/// Aircraft integrations should implement this with a source-issued boot UUID
/// or a persistent monotonic boot counter. The OS provider is appropriate for
/// simulator and development attachments.
pub trait IncarnationProvider {
    /// Returns the next attachment identity.
    ///
    /// The `_blocking` suffix makes the possible operating-system entropy I/O
    /// visible to callers.
    ///
    /// # Errors
    ///
    /// Returns a typed adapter error when the identity source is unavailable.
    fn next_incarnation_blocking(&mut self) -> Result<SourceIncarnation, AviateAdapterError>;
}

/// Operating-system CSPRNG identity provider for simulator attachments.
#[derive(Debug, Default)]
pub struct OsIncarnationProvider;

impl IncarnationProvider for OsIncarnationProvider {
    fn next_incarnation_blocking(&mut self) -> Result<SourceIncarnation, AviateAdapterError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|source| AviateAdapterError::IncarnationUnavailable { source })?;
        Ok(SourceIncarnation::new(bytes))
    }
}
