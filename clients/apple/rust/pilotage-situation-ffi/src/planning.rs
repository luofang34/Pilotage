//! Offline navigation search through verified installed source editions.

use pilotage_planning::{NavigationDataset, NavigationIndex, PlanningError};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[cfg(test)]
mod tests;

/// A navigation-search operation failed.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum PlanningFfiError {
    /// A planning record or index is invalid.
    #[error("{0}")]
    Planning(#[from] PlanningError),
    /// The search-session lock is unavailable.
    #[error("navigation search session is unavailable")]
    Unavailable,
    /// A source does not match its installed release.
    #[error("navigation source does not match release {release_id}")]
    Identity {
        /// The affected release.
        release_id: String,
    },
}

/// Calculates a resolved route without sending a vehicle command.
#[uniffi::export]
pub fn evaluate_planned_route(route_json: String) -> Result<String, PlanningFfiError> {
    let route = serde_json::from_str(&route_json).map_err(PlanningError::from)?;
    let summary = pilotage_planning::evaluate_route(&route)?;
    serde_json::to_string(&summary)
        .map_err(PlanningError::from)
        .map_err(Into::into)
}

/// Calculates individual and swarm timing without a vehicle side effect.
#[uniffi::export]
pub fn evaluate_coordinated_plan(plan_json: String) -> Result<String, PlanningFfiError> {
    let plan = serde_json::from_str(&plan_json).map_err(PlanningError::from)?;
    let summary = pilotage_planning::assess_coordination(&plan)?;
    serde_json::to_string(&summary)
        .map_err(PlanningError::from)
        .map_err(Into::into)
}

/// Exports a resolved local draft and its review digest. It does not authorize execution.
#[uniffi::export]
pub fn export_coordinated_plan(plan_json: String) -> Result<String, PlanningFfiError> {
    let plan = serde_json::from_str(&plan_json).map_err(PlanningError::from)?;
    Ok(pilotage_planning::export_revision(&plan)?)
}

/// Validates a portable vehicle profile document before import or local use.
#[uniffi::export]
pub fn validate_vehicle_profiles(document_json: String) -> Result<String, PlanningFfiError> {
    let document: pilotage_planning::VehicleProfileDocument =
        serde_json::from_str(&document_json).map_err(PlanningError::from)?;
    document.validate()?;
    serde_json::to_string(&document)
        .map_err(PlanningError::from)
        .map_err(Into::into)
}

/// One verified installed artifact to include in offline search.
#[derive(Clone, uniffi::Record)]
pub struct NavigationIndexRequest {
    /// Immutable package identity.
    pub release_id: String,
    /// Publishing authority and product.
    pub authority: String,
    /// Absolute path to a verified ACNAV or navigation SQLite artifact.
    pub path: String,
    /// Artifact format: `acnav` or `nav_sqlite`.
    pub format: String,
    /// First effective UTC instant in Unix seconds.
    pub effective_at: i64,
    /// First expired UTC instant in Unix seconds.
    pub expires_at: i64,
    /// Artifact SHA-256 from the verified release.
    pub artifact_digest: String,
}

/// A source set. Replacing sources succeeds for the complete set or leaves it unchanged.
#[derive(uniffi::Object)]
pub struct NavigationSearchSession {
    indexes: Mutex<Vec<NavigationIndex>>,
}

#[uniffi::export]
impl NavigationSearchSession {
    /// Creates an empty search session.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            indexes: Mutex::new(Vec::new()),
        })
    }

    /// Opens indexes off the UI thread and replaces the complete source set.
    pub fn replace_sources_blocking(
        &self,
        sources: Vec<NavigationIndexRequest>,
        cache_directory: String,
    ) -> Result<(), PlanningFfiError> {
        let mut indexes = Vec::new();
        for request in sources {
            indexes.push(open_index_blocking(&request, Path::new(&cache_directory))?);
        }
        *self
            .indexes
            .lock()
            .map_err(|_| PlanningFfiError::Unavailable)? = indexes;
        Ok(())
    }

    /// Searches selected sources and merges duplicate records of the same point.
    pub fn search_blocking(
        &self,
        query: String,
        limit: u32,
        now: i64,
    ) -> Result<String, PlanningFfiError> {
        let indexes = self
            .indexes
            .lock()
            .map_err(|_| PlanningFfiError::Unavailable)?;
        let mut matches = Vec::new();
        for index in indexes.iter() {
            matches.extend(index.search_blocking(&query, 100)?);
        }
        let matches = pilotage_planning::merge_navigation_matches(matches, &query, now, limit);
        serde_json::to_string(&matches)
            .map_err(PlanningError::from)
            .map_err(Into::into)
    }
}

fn open_index_blocking(
    request: &NavigationIndexRequest,
    cache: &Path,
) -> Result<NavigationIndex, PlanningFfiError> {
    let index = match request.format.as_str() {
        "nav_sqlite" => NavigationIndex::open_blocking(Path::new(&request.path))?,
        "acnav" => {
            std::fs::create_dir_all(cache).map_err(|source| PlanningError::File {
                path: cache.into(),
                source,
            })?;
            // The cache name depends only on the verified content digest.
            if request.artifact_digest.len() != 64
                || !request
                    .artifact_digest
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit())
            {
                return Err(PlanningFfiError::Identity {
                    release_id: request.release_id.clone(),
                });
            }
            let release_key = format!("{:x}", Sha256::digest(request.release_id.as_bytes()));
            let path = cache.join(format!(
                "nav-v1-{}-{}.sqlite",
                release_key, request.artifact_digest
            ));
            if path.exists() {
                NavigationIndex::open_blocking(&path)?
            } else {
                let bytes = std::fs::read(&request.path).map_err(|source| PlanningError::File {
                    path: request.path.clone().into(),
                    source,
                })?;
                let mut dataset =
                    NavigationDataset::from_acnav(request.release_id.clone(), &bytes)?;
                if dataset.source.source_digest != request.artifact_digest {
                    return Err(PlanningFfiError::Identity {
                        release_id: request.release_id.clone(),
                    });
                }
                // ACNAV stores dates. The verified release supplies each exact UTC instant.
                if dataset.source.effective_at.div_euclid(86400)
                    != request.effective_at.div_euclid(86400)
                    || dataset.source.expires_at.div_euclid(86400)
                        != request.expires_at.div_euclid(86400)
                {
                    return Err(PlanningFfiError::Identity {
                        release_id: request.release_id.clone(),
                    });
                }
                dataset.source.effective_at = request.effective_at;
                dataset.source.expires_at = request.expires_at;
                NavigationIndex::build_blocking(&path, &dataset)?
            }
        }
        _ => {
            return Err(PlanningFfiError::Identity {
                release_id: request.release_id.clone(),
            });
        }
    };
    let source = index.source();
    if source.release_id != request.release_id
        || source.authority != request.authority
        || source.effective_at != request.effective_at
        || source.expires_at != request.expires_at
    {
        return Err(PlanningFfiError::Identity {
            release_id: request.release_id.clone(),
        });
    }
    Ok(index)
}
