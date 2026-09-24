//! Refusals of the Aircraft domain.

use thiserror::Error;

/// Why a profile, loading, or calculation was refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AircraftError {
    /// A profile value is missing, non-finite, or out of range.
    #[error("invalid aircraft profile: {field}")]
    InvalidProfile {
        /// The refused field.
        field: &'static str,
    },
    /// A loading names a station or tank that the profile does not have, or a
    /// value outside its limit.
    #[error("invalid loading for {name}: {reason}")]
    InvalidLoading {
        /// The station or tank.
        name: String,
        /// Why the value was refused.
        reason: &'static str,
    },
    /// The loading was made for a different profile.
    #[error("loading is for profile {loading}, not {profile}")]
    ProfileMismatch {
        /// Profile hash that the loading names.
        loading: String,
        /// Profile hash of the calculation.
        profile: String,
    },
    /// The profile could not be encoded for hashing.
    #[error("cannot encode aircraft profile")]
    Encoding(#[source] serde_json::Error),
}
