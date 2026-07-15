//! The processed output elements, after conversion, clipping, interpolation, and
//! merging, before they are grouped into tiles and serialized.
//!
//! Every element carries the source record(s) it was derived from, so the
//! per-tile lineage the provenance records is a direct roll-up of these refs. An
//! element's coordinates are already in the target datum; the payload encoders
//! ([`crate::payload`]) turn a tile's elements into canonical bytes.

use crate::source::{ObstacleKind, SourceRecordRef};

/// A resampled terrain post: a node of the output grid with an elevation in the
/// target vertical datum, tracing to the source posts that fed its
/// interpolation.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputPost {
    /// Global grid row index (latitude), from the coverage south edge.
    pub i: u32,
    /// Global grid column index (longitude), from the coverage west edge.
    pub j: u32,
    /// Node latitude, target horizontal datum, degrees.
    pub lat_deg: f64,
    /// Node longitude, target horizontal datum, degrees.
    pub lon_deg: f64,
    /// Elevation, target vertical datum, meters.
    pub elevation_m: f64,
    /// The source posts (grid corners) this node was interpolated from, sorted.
    pub sources: Vec<SourceRecordRef>,
}

/// A merged output obstacle in the target datum, tracing to every source
/// obstacle that merged into it.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputObstacle {
    /// Latitude, target horizontal datum, degrees.
    pub lat_deg: f64,
    /// Longitude, target horizontal datum, degrees.
    pub lon_deg: f64,
    /// Above-ground-level height, meters.
    pub height_m: f64,
    /// The obstacle kind.
    pub kind: ObstacleKind,
    /// Every source obstacle that merged into this one, sorted.
    pub sources: Vec<SourceRecordRef>,
}

/// An output aerodrome reference point in the target datum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputAerodrome {
    /// The aerodrome identifier.
    pub ident: u32,
    /// Reference-point latitude, target horizontal datum, degrees.
    pub lat_deg: f64,
    /// Reference-point longitude, target horizontal datum, degrees.
    pub lon_deg: f64,
    /// Reference-point elevation, target vertical datum, meters.
    pub elevation_m: f64,
    /// The source record this came from.
    pub source: SourceRecordRef,
}

/// An output runway (its two ends) in the target datum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputRunway {
    /// The runway designator.
    pub designator: u16,
    /// End A latitude, target horizontal datum, degrees.
    pub end_a_lat_deg: f64,
    /// End A longitude, target horizontal datum, degrees.
    pub end_a_lon_deg: f64,
    /// End B latitude, target horizontal datum, degrees.
    pub end_b_lat_deg: f64,
    /// End B longitude, target horizontal datum, degrees.
    pub end_b_lon_deg: f64,
    /// The source record this came from.
    pub source: SourceRecordRef,
}
