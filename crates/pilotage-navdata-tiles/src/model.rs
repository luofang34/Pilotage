//! Public bundle and report types.

/// Drawable baseline records by layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct BaselineFeatureCounts {
    /// Aerodrome point subjects.
    pub aerodromes: u64,
    /// Ground navigation aid point subjects.
    pub navaids: u64,
    /// Published fix point subjects.
    pub fixes: u64,
    /// Airway subjects with at least one resolved segment.
    pub airways: u64,
    /// Airspace records with drawable published bounds.
    pub airspaces: u64,
}

impl BaselineFeatureCounts {
    /// Gets the total number of drawable baseline records.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.aerodromes + self.navaids + self.fixes + self.airways + self.airspaces
    }
}

/// Snapshot records that the first baseline schema cannot draw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct OmittedFeatureCounts {
    /// Point kinds that have no baseline layer.
    pub other_points: u64,
    /// Airways with fewer than one resolved segment.
    pub unresolved_airways: u64,
    /// Runways without WGS84 end positions in the snapshot model.
    pub runways_without_geometry: u64,
    /// Airspaces without a resolvable horizontal bound.
    pub airspaces_without_geometry: u64,
}

/// Deterministic facts produced with one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct NavdataTileReport {
    /// Number of populated vector tiles.
    pub tile_count: u64,
    /// Number of feature copies across all tiles.
    pub tile_feature_count: u64,
    /// Drawable baseline records by layer.
    pub features: BaselineFeatureCounts,
    /// Snapshot records that have no drawable baseline geometry.
    pub omitted: OmittedFeatureCounts,
    /// Complete archive size in bytes.
    pub archive_bytes: u64,
}

/// One deterministic MBTiles archive and its report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavdataTileBundle {
    pub(crate) bytes: Vec<u8>,
    pub(crate) report: NavdataTileReport,
}

impl NavdataTileBundle {
    /// Gets the complete MBTiles archive bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Takes the complete MBTiles archive bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Gets facts from this build.
    #[must_use]
    pub const fn report(&self) -> &NavdataTileReport {
        &self.report
    }
}
