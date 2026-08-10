//! Errors that reject an inconsistent Navdata input.

use thiserror::Error;

/// Failure to construct an identified Navdata snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AirspaceViewError {
    /// The supplied identity names a different cycle from the snapshot data.
    #[error(
        "Navdata identity cycle {identity_cycle:?} does not match snapshot cycle {snapshot_cycle:?}"
    )]
    SnapshotCycleMismatch {
        /// Cycle in the supplied identity.
        identity_cycle: String,
        /// Cycle derived from the snapshot.
        snapshot_cycle: String,
    },
}
