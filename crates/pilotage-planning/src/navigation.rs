//! Source-neutral navigation records and a derived search database.

mod database;
mod search;
mod snapshot;

use crate::{PlanningError, error::invalid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use database::NavigationIndex;
pub use search::merge_navigation_matches;

/// The exact source edition used to resolve a navigation point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationSource {
    /// Immutable release identifier.
    pub release_id: String,
    /// Publishing authority and product, such as `faa-nasr`.
    pub authority: String,
    /// Source edition name.
    pub edition: String,
    /// SHA-256 digest of the input source bytes, in lowercase hexadecimal.
    pub source_digest: String,
    /// First effective UTC instant, in Unix seconds.
    pub effective_at: i64,
    /// First expired UTC instant, in Unix seconds.
    pub expires_at: i64,
}

/// A point in a source edition. Coordinates use WGS 84.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationPoint {
    /// A record key unique within the source edition.
    pub key: String,
    /// Published identifier.
    pub identifier: String,
    /// Point type: `airport`, `waypoint`, or `navaid`.
    pub kind: String,
    /// Published name. Empty when the source has no name.
    pub name: String,
    /// Published region. Empty when the source has no region.
    pub region: String,
    /// Latitude in degrees.
    pub latitude_deg: f64,
    /// Longitude in degrees.
    pub longitude_deg: f64,
}

/// Normalized input accepted from a source converter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationDataset {
    /// This implementation accepts version 1.
    pub schema_version: u32,
    /// Exact source identity and validity.
    pub source: NavigationSource,
    /// Searchable records from that source.
    pub points: Vec<NavigationPoint>,
}

/// One search match with the source required to retain it in a plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Resolved point, including its source record key.
    pub point: NavigationPoint,
    /// The edition that supplied this point.
    pub source: NavigationSource,
}

impl NavigationSource {
    /// Checks the source identity and half-open validity interval.
    pub fn validate(&self) -> Result<(), PlanningError> {
        for (field, value) in [
            ("release_id", &self.release_id),
            ("authority", &self.authority),
            ("edition", &self.edition),
        ] {
            text(field, value)?;
        }
        if self.source_digest.len() != 64
            || !self.source_digest.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(invalid("source_digest", "expected a SHA-256 digest"));
        }
        if self.effective_at >= self.expires_at {
            return Err(invalid("validity", "effective time must precede expiry"));
        }
        Ok(())
    }
}

impl NavigationPoint {
    /// Checks the identifier, point type, and coordinate range.
    pub fn validate(&self) -> Result<(), PlanningError> {
        text("point.key", &self.key)?;
        text("point.identifier", &self.identifier)?;
        if !matches!(self.kind.as_str(), "airport" | "waypoint" | "navaid") {
            return Err(invalid(&self.key, "unsupported point type"));
        }
        if !(-90.0..=90.0).contains(&self.latitude_deg)
            || !(-180.0..=180.0).contains(&self.longitude_deg)
        {
            return Err(invalid(&self.key, "coordinate is outside WGS 84 limits"));
        }
        if self.name.len() > 1024 || self.region.len() > 256 {
            return Err(invalid(&self.key, "name or region is too long"));
        }
        Ok(())
    }
}

impl NavigationDataset {
    /// Checks the schema and all records before an index is written.
    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.schema_version != 1 {
            return Err(invalid("schema_version", "expected 1"));
        }
        self.source.validate()?;
        let mut keys = BTreeSet::new();
        for point in &self.points {
            point.validate()?;
            if !keys.insert(&point.key) {
                return Err(invalid(&point.key, "duplicate source record key"));
            }
        }
        Ok(())
    }
}

pub(crate) fn text(field: &str, value: &str) -> Result<(), PlanningError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid(
            field,
            "expected 1 to 256 bytes without control characters",
        ));
    }
    Ok(())
}
