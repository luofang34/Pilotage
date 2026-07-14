//! The source-record model the chain ingests: terrain grids, obstacles, and
//! aerodromes, each carrying the identity of the source it came from.
//!
//! Source data is stated in its own reference frame: each [`SourceMeta`] records
//! the horizontal and vertical datum, the identities those datums need, the
//! license the data is used under, and the accuracy the source claims. Every
//! output element carries a [`SourceRecordRef`] back to the exact record it was
//! derived from, so the chain can prove input-to-output traceability. This model
//! is deliberately small and synthetic; large or license-restricted datasets are
//! never checked in.

mod digest;
mod license;

#[cfg(test)]
mod tests;

pub(crate) use digest::source_content_digest;
pub use license::LicenseCode;

use pilotage_geo::{DatumRealizationId, GeoidModelId, HorizontalDatum, VerticalDatum};
use pilotage_svs_db::Accuracy;

/// Identity of a source dataset feeding the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct SourceId(
    /// The opaque source identifier.
    pub u32,
);

/// A back-reference from an output element to the exact source record it came
/// from. Ordered so provenance can list contributors in a stable order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct SourceRecordRef {
    /// The source the record came from.
    pub source: SourceId,
    /// The record's index within that source.
    pub record: u32,
}

/// The reference frame, license, and quality a source states for its records.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceMeta {
    /// The source this metadata describes.
    pub id: SourceId,
    /// The immutable version of this source input. Bound, with the content
    /// digest, into the signed provenance so a changed input is a changed
    /// version.
    pub version: u32,
    /// The license the data is used under, which maps to package use
    /// restrictions.
    pub license: LicenseCode,
    /// The horizontal datum the source's coordinates are stated in.
    pub horizontal_datum: HorizontalDatum,
    /// The horizontal-datum realization; `UNDECLARED` when none is needed.
    pub realization: DatumRealizationId,
    /// The vertical datum the source's heights are stated in.
    pub vertical_datum: VerticalDatum,
    /// The geoid model behind a geometric-MSL source; `UNDECLARED` otherwise.
    pub geoid: GeoidModelId,
    /// The accuracy the source claims for its data.
    pub accuracy: Accuracy,
}

/// The kind of a vertical obstacle. The discriminant is the wire encoding and
/// the merge tie-break key, so merging is deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[repr(u8)]
pub enum ObstacleKind {
    /// A tower (broadcast, lattice).
    Tower = 1,
    /// A guyed mast.
    Mast = 2,
    /// A building.
    Building = 3,
    /// A crane (often temporary).
    Crane = 4,
    /// A wind turbine.
    WindTurbine = 5,
}

impl ObstacleKind {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
}

/// A vertical obstacle: a position, an above-ground-level height, a kind, and
/// its source. The height is a length above ground and so is datum-independent;
/// the position is stated in the source horizontal datum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obstacle {
    /// Latitude in the source horizontal datum, degrees.
    pub lat_deg: f64,
    /// Longitude in the source horizontal datum, degrees.
    pub lon_deg: f64,
    /// Above-ground-level height of the obstacle, meters. Must be positive.
    pub height_m: f64,
    /// The kind of obstacle.
    pub kind: ObstacleKind,
    /// The record this obstacle came from.
    pub source: SourceRecordRef,
}

/// A regular terrain elevation grid stated in a source's frame. Nodes are
/// row-major with latitude on the outer axis and longitude on the inner axis,
/// starting at the south-west origin. A `None` post is a void (a hole) the chain
/// must resolve by policy rather than invent a value for.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainGrid {
    /// The source this grid came from.
    pub source: SourceId,
    /// Latitude of node row 0 (south edge), degrees, in the source datum.
    pub origin_lat_deg: f64,
    /// Longitude of node column 0 (west edge), degrees, in the source datum.
    pub origin_lon_deg: f64,
    /// Node spacing, degrees. Must be positive and finite.
    pub step_deg: f64,
    /// Number of node rows (along latitude).
    pub rows: u32,
    /// Number of node columns (along longitude).
    pub cols: u32,
    /// Row-major posts (`rows * cols`), heights in the source vertical datum;
    /// `None` marks a void.
    pub posts: Vec<Option<f64>>,
}

impl TerrainGrid {
    /// The latitude of node row `r`, in the source datum.
    #[must_use]
    pub fn node_lat_deg(&self, r: u32) -> f64 {
        self.origin_lat_deg + f64::from(r) * self.step_deg
    }

    /// The longitude of node column `c`, in the source datum.
    #[must_use]
    pub fn node_lon_deg(&self, c: u32) -> f64 {
        self.origin_lon_deg + f64::from(c) * self.step_deg
    }

    /// The post at `(r, c)`, or `None` for a void or an out-of-range index.
    #[must_use]
    pub fn post(&self, r: u32, c: u32) -> Option<f64> {
        if r >= self.rows || c >= self.cols {
            return None;
        }
        let idx = (r as usize)
            .checked_mul(self.cols as usize)?
            .checked_add(c as usize)?;
        self.posts.get(idx).copied().flatten()
    }

    /// The record index of node `(r, c)` within this source, for provenance.
    #[must_use]
    pub fn record_index(&self, r: u32, c: u32) -> u32 {
        r.wrapping_mul(self.cols).wrapping_add(c)
    }
}

/// A runway, as its two end points, in the source horizontal datum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Runway {
    /// The runway designator (e.g. packed heading pair).
    pub designator: u16,
    /// Latitude of end A, degrees.
    pub end_a_lat_deg: f64,
    /// Longitude of end A, degrees.
    pub end_a_lon_deg: f64,
    /// Latitude of end B, degrees.
    pub end_b_lat_deg: f64,
    /// Longitude of end B, degrees.
    pub end_b_lon_deg: f64,
    /// The record this runway came from.
    pub source: SourceRecordRef,
}

/// An aerodrome: a reference point, an elevation, its runways, and its source.
#[derive(Debug, Clone, PartialEq)]
pub struct Aerodrome {
    /// A stable numeric identifier (e.g. packed ICAO code).
    pub ident: u32,
    /// Reference-point latitude, source horizontal datum, degrees.
    pub ref_lat_deg: f64,
    /// Reference-point longitude, source horizontal datum, degrees.
    pub ref_lon_deg: f64,
    /// Reference-point elevation, source vertical datum, meters.
    pub elevation_m: f64,
    /// The record this aerodrome came from.
    pub source: SourceRecordRef,
    /// The aerodrome's runways.
    pub runways: Vec<Runway>,
}

/// The full source dataset the chain ingests.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceDataset {
    /// Per-source metadata (datum, license, accuracy).
    pub meta: Vec<SourceMeta>,
    /// Terrain grids.
    pub terrain: Vec<TerrainGrid>,
    /// Vertical obstacles.
    pub obstacles: Vec<Obstacle>,
    /// Aerodromes.
    pub aerodromes: Vec<Aerodrome>,
}

impl SourceDataset {
    /// The metadata for `id`, if the dataset declares it.
    #[must_use]
    pub fn meta_for(&self, id: SourceId) -> Option<&SourceMeta> {
        self.meta.iter().find(|m| m.id == id)
    }
}
