//! Mission navdata loading at the binary edge (ADR-0030): the one place a
//! snapshot enters the process, always through the versioned blob
//! container's decode-and-verify path.
//!
//! Fixture mode packs the generated demo snapshot through the real
//! container and decodes it back, so the demo travels exactly the road
//! published data does. Store mode scans a directory of published
//! `*/*.acnav` blobs, keeps the cycles whose effectivity covers the
//! flight date, prefers the `faa-nasr` authority, and lets the newest
//! effective date win.

use std::path::{Path, PathBuf};

use aerocontext_core::NavDataSnapshot;
use aerocontext_navdata::blob;
use chrono::NaiveDate;
use pilotage_mission::{MissionBuildError, SnapshotProvenance, decode_snapshot};
use tracing::info;

use crate::runtime::{MissionNavdataSource, MissionOptions};

/// The authority slug preferred when a store holds several covering cycles.
const PREFERRED_AUTHORITY: &str = "faa-nasr";

/// Why the mission navdata could not be loaded or selected.
#[derive(Debug, thiserror::Error)]
pub enum NavdataError {
    /// The fixture snapshot could not be generated or round-tripped.
    /// Boxed: the build error carries route/cycle context and would
    /// otherwise dominate every `Result` on the startup path.
    #[error("fixture navdata failed: {0}")]
    Fixture(#[source] Box<MissionBuildError>),
    /// A store directory or blob file could not be read.
    #[error("failed to read {path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A `.acnav` blob failed the container's verify-and-decode.
    #[error("navdata blob {path} rejected: {source}")]
    Blob {
        /// The rejected blob's path.
        path: PathBuf,
        /// The container failure.
        #[source]
        source: Box<blob::BlobError>,
    },
    /// The selected blob decoded at inspect time but not at load time.
    #[error("navdata blob {path} failed to decode: {source}")]
    Decode {
        /// The failing blob's path.
        path: PathBuf,
        /// The decode failure.
        #[source]
        source: Box<MissionBuildError>,
    },
    /// No blob in the store covers the flight date.
    #[error("no cycle in store {store} covers {date}")]
    NoCoveringCycle {
        /// The scanned store directory.
        store: PathBuf,
        /// The uncovered flight date.
        date: NaiveDate,
    },
}

/// A decoded, provenance-bound snapshot ready for mission planning.
#[derive(Debug, Clone)]
pub struct LoadedNavdata {
    /// The decoded snapshot.
    pub snapshot: NavDataSnapshot,
    /// The blob-verified provenance record (ADR-0030).
    pub provenance: SnapshotProvenance,
}

/// Loads the snapshot the mission options name.
///
/// # Errors
///
/// Returns [`NavdataError`] when the fixture cannot be generated, the
/// store cannot be read, a blob fails verification, or no cycle covers
/// the flight date.
pub fn load(options: &MissionOptions) -> Result<LoadedNavdata, NavdataError> {
    match &options.navdata {
        MissionNavdataSource::Fixture => {
            if let Some(date) = options.date {
                info!(%date, "PILOTAGE_MISSION_DATE ignored: the fixture cycle is fixed");
            }
            let blob = pilotage_mission::fixture::demo_blob(options.anchor)
                .map_err(|source| NavdataError::Fixture(Box::new(source)))?;
            let (snapshot, provenance) = decode_snapshot(&blob, true)
                .map_err(|source| NavdataError::Fixture(Box::new(source)))?;
            Ok(LoadedNavdata {
                snapshot,
                provenance,
            })
        }
        MissionNavdataSource::Store(store) => {
            // Parse enforcement: a store always carries a flight date.
            let date = options.date.unwrap_or(NaiveDate::MIN);
            load_from_store(store, date)
        }
    }
}

/// One store blob whose effectivity covers the flight date.
struct Candidate {
    path: PathBuf,
    authority: String,
    effective_on: NaiveDate,
    bytes: Vec<u8>,
}

/// Scans `{store}/*/*.acnav`, keeps cycles covering `date`, prefers
/// [`PREFERRED_AUTHORITY`], newest `effective_on` winning, and decodes the
/// selected blob through the same verify path the fixture uses.
fn load_from_store(store: &Path, date: NaiveDate) -> Result<LoadedNavdata, NavdataError> {
    let mut best: Option<Candidate> = None;
    for path in store_blob_paths(store)? {
        let bytes = std::fs::read(&path).map_err(|source| NavdataError::Io {
            path: path.clone(),
            source,
        })?;
        let info = blob::inspect(&bytes).map_err(|source| NavdataError::Blob {
            path: path.clone(),
            source: Box::new(source),
        })?;
        let cycle = &info.snapshot.cycle;
        if !(cycle.effective_on <= date && date < cycle.next_effective_on) {
            continue;
        }
        let candidate = Candidate {
            path,
            authority: cycle.authority.slug().to_owned(),
            effective_on: cycle.effective_on,
            bytes,
        };
        if best
            .as_ref()
            .is_none_or(|current| candidate_outranks(&candidate, current))
        {
            best = Some(candidate);
        }
    }
    let Some(chosen) = best else {
        return Err(NavdataError::NoCoveringCycle {
            store: store.to_path_buf(),
            date,
        });
    };
    info!(path = %chosen.path.display(), authority = %chosen.authority,
        effective_on = %chosen.effective_on, "mission navdata selected from store");
    let (snapshot, provenance) =
        decode_snapshot(&chosen.bytes, false).map_err(|source| NavdataError::Decode {
            path: chosen.path,
            source: Box::new(source),
        })?;
    Ok(LoadedNavdata {
        snapshot,
        provenance,
    })
}

/// Preferred-authority first, then the newest effective date.
fn candidate_outranks(candidate: &Candidate, current: &Candidate) -> bool {
    let candidate_preferred = candidate.authority == PREFERRED_AUTHORITY;
    let current_preferred = current.authority == PREFERRED_AUTHORITY;
    if candidate_preferred != current_preferred {
        return candidate_preferred;
    }
    candidate.effective_on > current.effective_on
}

/// Every `{store}/*/*.acnav` path, in directory-listing order.
fn store_blob_paths(store: &Path) -> Result<Vec<PathBuf>, NavdataError> {
    let read_dir = |path: &Path| {
        std::fs::read_dir(path).map_err(|source| NavdataError::Io {
            path: path.to_path_buf(),
            source,
        })
    };
    let mut paths = Vec::new();
    for entry in read_dir(store)? {
        let entry = entry.map_err(|source| NavdataError::Io {
            path: store.to_path_buf(),
            source,
        })?;
        let subdir = entry.path();
        if !subdir.is_dir() {
            continue;
        }
        for file in read_dir(&subdir)? {
            let file = file.map_err(|source| NavdataError::Io {
                path: subdir.clone(),
                source,
            })?;
            let path = file.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "acnav")
            {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}
