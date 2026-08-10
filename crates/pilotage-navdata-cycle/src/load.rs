use std::path::{Path, PathBuf};

use aerocontext_core::NavDataSnapshot;

/// A published cycle could not become a snapshot.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CycleLoadError {
    /// The cycle file could not be read.
    #[error("cycle file {path} could not be read")]
    Read {
        /// Path this load was asked for.
        path: PathBuf,
        /// Underlying file failure.
        #[source]
        source: std::io::Error,
    },
    /// The bytes are not an encoded cycle this build reads.
    #[error("cycle file {path} does not hold a navigation-data snapshot")]
    Decode {
        /// Path this load was asked for.
        path: PathBuf,
        /// Underlying decode failure.
        #[source]
        source: aerocontext_navdata::BlobError,
    },
}

/// Read one encoded cycle from disk.
///
/// The caller names the file. A cycle is chosen by a subscription and a date elsewhere,
/// and a loader that also chose would hide which cycle a run used.
pub fn load_cycle(path: impl AsRef<Path>) -> Result<NavDataSnapshot, CycleLoadError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| CycleLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    load_cycle_bytes(path, &bytes)
}

/// Decode one encoded cycle already in memory.
///
/// An iOS client receives the cycle as an asset rather than as a path, so the decode step
/// stands on its own. The path is carried for the error alone.
pub fn load_cycle_bytes(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<NavDataSnapshot, CycleLoadError> {
    aerocontext_navdata::decode(bytes).map_err(|source| CycleLoadError::Decode {
        path: path.as_ref().to_path_buf(),
        source,
    })
}
