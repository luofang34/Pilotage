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
    /// A profile station or tank is refused.
    #[error("invalid aircraft profile entry {name:?}: {reason}")]
    InvalidProfileEntry {
        /// The station or tank.
        name: String,
        /// Why the entry was refused.
        reason: &'static str,
    },
    /// A loading names a station or tank that the profile does not have, or a
    /// value outside its limit.
    #[error("invalid loading for {name}: {reason}")]
    InvalidLoading {
        /// The station, the tank, or the loading field.
        name: String,
        /// Why the value was refused.
        reason: &'static str,
    },
    /// An energy state for a calculation is non-finite, negative, or larger
    /// than the store holds.
    #[error("invalid energy state: {reason}")]
    InvalidEnergyState {
        /// Why the state was refused.
        reason: &'static str,
    },
    /// A fuel quantity was offered for a battery aircraft, or the reverse.
    #[error("the profile stores {store} energy, not {offered}")]
    EnergyKindMismatch {
        /// Energy store kind of the profile.
        store: &'static str,
        /// Energy kind that was offered.
        offered: &'static str,
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
