//! Structured, serializable build provenance: the complete input-to-output
//! lineage of a package.
//!
//! Provenance records, for the whole build, the tool identity and version, the
//! parameters, a per-source summary (including license and datum), the ordered
//! stages with their input/output/rejected counts, a change report of every
//! changed record's disposition (rejected as an outlier, clipped, or merged), a
//! per-tile roll-up of contributing sources, and a per-record lineage tracing
//! each emitted output record back to the specific source record(s) that
//! produced it. It closes with the content hash of the package it describes,
//! binding the lineage to the exact bytes signed. Everything serializes
//! deterministically (sorted keys, no maps), so the provenance of a reproduced
//! build is byte-identical too.
//!
//! # SIM / NOT FOR FLIGHT
//!
//! Provenance is engineering evidence of how the chain ran. It is not approved
//! aeronautical data and asserts no certification, compliance, or airworthiness.

use serde::{Serialize, Serializer};

use crate::source::{LicenseCode, SourceId, SourceRecordRef};

/// The build tool's stable identity, written into the signed processing chain.
pub const TOOL_ID: u32 = 0x5653_4233;

/// The build tool's version `(major, minor, patch)`.
pub const TOOL_VERSION: (u16, u16, u16) = (0, 1, 0);

/// What changed for one source record in the change report. Records that
/// contributed unchanged to an emitted tile are traced through the per-tile
/// lineage rather than repeated here, so this report is exactly the set of
/// changes an update review must reason about.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Disposition {
    /// The record was rejected as an outlier.
    RejectedOutlier {
        /// Why the record was rejected.
        reason: &'static str,
    },
    /// The record fell outside the coverage box and was clipped.
    Clipped,
    /// The record was consumed but contributed no output (every derived node
    /// stayed void, or every candidate lost deterministic resolution) — its
    /// fate is recorded so no consumed source can look like a phantom.
    NoContribution {
        /// Why nothing survived.
        reason: &'static str,
    },
    /// An obstacle merged into another obstacle at the given position.
    Merged {
        /// Latitude of the obstacle it merged into, degrees.
        into_lat_deg: f64,
        /// Longitude of the obstacle it merged into, degrees.
        into_lon_deg: f64,
    },
}

/// One record's disposition in the change report.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RecordDisposition {
    /// The source record.
    pub source: SourceRecordRef,
    /// What became of it.
    pub disposition: Disposition,
}

/// One processing stage's summary: what it consumed, produced, and rejected.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct StageRecord {
    /// The stage's operation code, matching the signed processing chain.
    pub code: u16,
    /// A stable human-readable stage name.
    pub name: &'static str,
    /// Elements the stage consumed.
    pub inputs: u32,
    /// Elements the stage produced.
    pub outputs: u32,
    /// Elements the stage rejected or dropped.
    pub rejected: u32,
}

/// A per-source summary: identity, immutable version and content digest,
/// license, datum, accuracy, and record count. The version and digest bind the
/// exact source input into the signed provenance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SourceSummary {
    /// The source identity.
    pub id: SourceId,
    /// The immutable version of the source input.
    pub version: u32,
    /// The SHA-256 content digest of the source input, hex-encoded.
    #[serde(serialize_with = "serialize_hex32")]
    pub content_digest: [u8; 32],
    /// The license the source's data is used under.
    pub license: LicenseCode,
    /// The source horizontal datum wire code.
    pub horizontal_datum: u8,
    /// The source vertical datum wire code.
    pub vertical_datum: u8,
    /// Horizontal 1-sigma accuracy, millimeters.
    pub accuracy_h_mm: u32,
    /// Vertical 1-sigma accuracy, millimeters.
    pub accuracy_v_mm: u32,
    /// Number of records the source supplied.
    pub record_count: u32,
}

/// The lineage of one emitted tile: its identity and the source records that
/// contributed to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TileLineage {
    /// The tile's feature-class wire code.
    pub class: u8,
    /// Tile latitude index.
    pub lat_index: i32,
    /// Tile longitude index.
    pub lon_index: i32,
    /// Number of elements in the tile.
    pub element_count: u32,
    /// The distinct source records that contributed, sorted.
    pub sources: Vec<SourceRecordRef>,
}

/// A decodable identity for one emitted output record within its tile, so a
/// record's lineage can be resolved back to the record actually present in the
/// package. The fields are exactly the identity fields the tile payload encodes,
/// so a decoder can find the same record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum RecordKey {
    /// A terrain post at its global grid indices.
    TerrainNode {
        /// Global grid row index (latitude).
        i: u32,
        /// Global grid column index (longitude).
        j: u32,
    },
    /// An obstacle at a position and kind (position as IEEE-754 bit patterns).
    Obstacle {
        /// Latitude bit pattern.
        lat_bits: u64,
        /// Longitude bit pattern.
        lon_bits: u64,
        /// Obstacle kind wire code.
        kind: u8,
    },
    /// An aerodrome reference point, by identifier.
    Aerodrome {
        /// The aerodrome identifier.
        ident: u32,
    },
    /// A runway, by designator and end-A position (bit patterns).
    Runway {
        /// Runway designator.
        designator: u16,
        /// End-A latitude bit pattern.
        end_a_lat_bits: u64,
        /// End-A longitude bit pattern.
        end_a_lon_bits: u64,
    },
}

/// The lineage of one emitted output record: its tile, its decodable identity,
/// and the exact source record(s) it was derived from. This is the record-level
/// traceability the standard requires — a single output post, obstacle, or
/// runway resolves to the specific source records that produced it, not merely a
/// tile-wide aggregate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordLineage {
    /// The feature-class wire code.
    pub class: u8,
    /// Tile latitude index.
    pub lat_index: i32,
    /// Tile longitude index.
    pub lon_index: i32,
    /// The record's decodable identity within the tile.
    pub key: RecordKey,
    /// The source record(s) this output record was derived from, sorted.
    pub sources: Vec<SourceRecordRef>,
}

/// The numeric parameters the build ran under, for the provenance record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ParamSnapshot {
    /// Tile size, degrees.
    pub tile_deg: f64,
    /// Output post spacing, degrees.
    pub post_spacing_deg: f64,
    /// Recorded post spacing, millimeters.
    pub post_spacing_mm: u32,
    /// Lowest plausible elevation, meters.
    pub elevation_min_m: f64,
    /// Highest plausible elevation, meters.
    pub elevation_max_m: f64,
    /// Highest plausible obstacle height, meters.
    pub max_obstacle_height_m: f64,
    /// Largest interpolated void span, source nodes.
    pub max_hole_span: u32,
    /// Obstacle merge tolerance, degrees.
    pub merge_tolerance_deg: f64,
    /// Declared integrity level wire code.
    pub integrity: u8,
    /// Target horizontal datum wire code.
    pub target_horizontal: u8,
    /// Target horizontal realization id.
    pub target_realization: u16,
    /// Target vertical datum wire code.
    pub target_vertical: u8,
    /// Target geoid model id.
    pub target_geoid: u16,
    /// Effective day number.
    pub effective_day: u32,
    /// Expiry day number.
    pub expiry_day: u32,
    /// Release day number.
    pub release_day: u32,
}

/// The complete structured provenance of a build.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BuildProvenance {
    /// The build tool identity.
    pub tool_id: u32,
    /// The build tool version `(major, minor, patch)`.
    pub tool_version: (u16, u16, u16),
    /// The parameters the build ran under.
    pub params: ParamSnapshot,
    /// Per-source summaries, sorted by source id.
    pub sources: Vec<SourceSummary>,
    /// The ordered processing stages.
    pub stages: Vec<StageRecord>,
    /// The change report: every record's disposition, sorted.
    pub dispositions: Vec<RecordDisposition>,
    /// Per-tile lineage, sorted by tile identity.
    pub tiles: Vec<TileLineage>,
    /// Per-record lineage: every emitted output record traced to its source
    /// record(s), sorted by class then tile then record key.
    pub records: Vec<RecordLineage>,
    /// The content hash of the package this provenance describes, hex-encoded,
    /// binding the lineage to the exact signed bytes.
    #[serde(serialize_with = "serialize_hex32")]
    pub package_content_hash: [u8; 32],
}

/// Serializes a 32-byte hash as a lowercase hex string, so provenance carries a
/// stable, readable binding to the package it describes.
fn serialize_hex32<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
    let mut hex = String::with_capacity(64);
    for byte in bytes {
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    serializer.serialize_str(&hex)
}
