use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ContentDigest, PackageError, PackageId, PackagePath, error::invalid};

/// A product with its own release schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    /// Cycle-dated navigation records.
    Navdata,
    /// Elevation data.
    Terrain,
    /// Low-altitude instrument charts.
    IfrLow,
    /// High-altitude instrument charts.
    IfrHigh,
    /// Terminal procedure records and documents.
    Procedures,
    /// Geographic background tiles.
    Basemap,
}

/// The declared publication channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// A release admitted by the publisher for normal selection.
    Stable,
    /// A release for development and inspection.
    Development,
}

/// Permission to redistribute the package bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    /// The publisher records permission to redistribute these bytes.
    Permitted,
    /// Access terms prohibit public redistribution.
    Restricted,
    /// Distribution permission has not been established.
    #[default]
    Unspecified,
}

/// The byte format of one installed artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    /// An AeroContext navigation snapshot.
    Acnav,
    /// A versioned navigation query database.
    NavSqlite,
    /// A version 3 PMTiles archive.
    Pmtiles,
    /// An MBTiles archive.
    Mbtiles,
    /// A MapLibre style document.
    MapStyle,
    /// An indexed terminal procedure database.
    ProcedureSqlite,
    /// A procedure chart or other reference document.
    Pdf,
    /// A display resource named by a style.
    Resource,
}

/// A half-open interval of Unix seconds in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    /// First valid instant.
    pub effective_at: i64,
    /// First invalid instant.
    pub expires_at: i64,
}

impl Validity {
    /// Whether this interval contains an instant.
    pub fn contains(&self, utc: i64) -> bool {
        self.effective_at <= utc && utc < self.expires_at
    }
}

/// Declared geographic coverage and display detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    /// A name displayed to the reader.
    pub name: String,
    /// West, south, east, and north limits in degrees.
    pub bounds: [f64; 4],
    /// Lowest supported display zoom.
    pub min_zoom: u8,
    /// Highest supported display zoom.
    pub max_zoom: u8,
    /// Whether the package covers all required data within these limits.
    pub complete: bool,
    /// Known exclusions or sample limits.
    pub exclusions: Vec<String>,
}

/// One immutable downloadable file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// Path under the installed release directory.
    pub path: PackagePath,
    /// Absolute HTTPS URL or path relative to the catalog directory.
    pub source: String,
    /// Required artifact format.
    pub format: ArtifactFormat,
    /// Exact byte length.
    pub bytes: u64,
    /// Digest of the complete file.
    pub sha256: ContentDigest,
}

/// A complete immutable release of one product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Contract version. This implementation accepts version 1.
    pub schema_version: u32,
    /// Immutable release identity.
    pub id: PackageId,
    /// Product category.
    pub product: Product,
    /// Source authority or provider.
    pub authority: String,
    /// Revision within a product edition.
    pub revision: u64,
    /// Human-readable source edition.
    pub edition: String,
    /// Exact source-set identity.
    pub source_set: ContentDigest,
    /// Publication channel.
    pub channel: Channel,
    /// Product validity in UTC. Geographic products can have no declared expiry.
    pub validity: Option<Validity>,
    /// Geographic and detail limits.
    pub coverage: Coverage,
    /// Required immutable files.
    pub artifacts: Vec<Artifact>,
    /// Exact releases required with this product.
    pub dependencies: Vec<PackageId>,
    /// Required renderer capability names.
    pub renderer_capabilities: Vec<String>,
    /// Source and resource distribution notices.
    pub attributions: Vec<String>,
    /// Distribution permission for the complete artifact set.
    #[serde(default)]
    pub distribution: Distribution,
}

impl Release {
    /// Whether the product has no validity limit or contains the requested instant.
    pub fn valid_at(&self, utc: i64) -> bool {
        self.validity.is_none_or(|validity| validity.contains(utc))
    }

    /// Check structural invariants before installation or selection.
    pub fn validate(&self) -> Result<(), PackageError> {
        if self.schema_version != 1 {
            return Err(invalid("schema_version", self.schema_version));
        }
        if self.authority.is_empty() || self.edition.is_empty() {
            return Err(invalid("identity", "authority and edition are required"));
        }
        if self
            .validity
            .is_some_and(|v| v.effective_at >= v.expires_at)
            || (self.validity.is_none()
                && !matches!(self.product, Product::Terrain | Product::Basemap))
        {
            return Err(invalid("validity", self.id.as_str()));
        }
        self.coverage.validate()?;
        if self.artifacts.is_empty() {
            return Err(invalid("artifacts", "empty release"));
        }
        let mut paths = BTreeSet::new();
        let mut sizes = std::collections::BTreeMap::new();
        for artifact in &self.artifacts {
            if !paths.insert(artifact.path.as_str().to_ascii_lowercase()) {
                return Err(invalid("artifact_path", artifact.path.as_str()));
            }
            if artifact.bytes == 0 {
                return Err(invalid("artifact_bytes", artifact.path.as_str()));
            }
            if let Some(previous) = sizes.insert(&artifact.sha256, artifact.bytes)
                && previous != artifact.bytes
            {
                return Err(invalid("artifact_bytes", artifact.sha256.as_str()));
            }
            if !artifact.source.starts_with("https://") {
                PackagePath::try_from(artifact.source.clone())?;
            }
        }
        if self.dependencies.contains(&self.id)
            || self.dependencies.iter().collect::<BTreeSet<_>>().len() != self.dependencies.len()
        {
            return Err(invalid("dependencies", self.id.as_str()));
        }
        Ok(())
    }
}

impl Coverage {
    fn validate(&self) -> Result<(), PackageError> {
        let [west, south, east, north] = self.bounds;
        if !self.bounds.iter().all(|v| v.is_finite())
            || west < -180.0
            || east > 180.0
            || south < -90.0
            || north > 90.0
            || west >= east
            || south >= north
            || self.min_zoom > self.max_zoom
            || self.max_zoom > 30
            || self.name.is_empty()
        {
            return Err(invalid("coverage", &self.name));
        }
        if !self.complete && self.exclusions.is_empty() {
            return Err(invalid(
                "coverage_exclusions",
                "incomplete coverage requires a reason",
            ));
        }
        Ok(())
    }
}
