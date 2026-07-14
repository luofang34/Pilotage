//! Deterministic, offline terrain/obstacle/aerodrome processing chain that emits
//! a signed, provenance-preserving SVS-02 package (SVS-03).
//!
//! [`build_package`] turns a [`SourceDataset`] and a fixed [`BuildConfig`] into a
//! [`BuildArtifact`]: a signed [`pilotage_svs_db::CandidatePackage`] that the
//! SVS-02 verifier accepts, the structured [`BuildProvenance`] tracing every
//! output tile to its source records, and the coverage/quality/seam/hole
//! [`BuildReports`]. The chain is pure and deterministic — no wall clock, no
//! randomness, stable ordering everywhere — so a fixed input and toolchain
//! reproduce byte-identical package bytes (see [`canonical_package_bytes`]). Any
//! stage failure returns a [`BuildError`] and emits nothing, so a tool fault can
//! never activate a partial package. [`SemanticDiff`] turns a database update
//! into an auditable set of tile changes rather than an opaque new binary.
//!
//! All package types are consumed from `pilotage-svs-db` and all datum types
//! from `pilotage-geo`; this crate re-mints neither the package format nor the
//! datum vocabulary.
//!
//! # SIM / NOT FOR FLIGHT
//!
//! The output is engineering evidence of how the chain ran. It is not approved
//! aeronautical data: nothing here is certified, approved, or airworthy, and
//! passing these tests makes no compliance or airworthiness claim.

#![forbid(unsafe_code)]

mod bundle;
mod chain;
mod config;
mod datum;
mod diff;
mod element;
mod error;
mod package;
mod payload;
mod provenance;
mod report;
mod source;
mod verify;

#[cfg(test)]
mod fixtures;

pub use bundle::canonical_bundle_bytes;
pub use chain::{BuildArtifact, build_package};
pub use config::{BuildConfig, ChainParams, PackageIdentity, SigningConfig, TargetDatum};
pub use datum::{convert_horizontal, convert_vertical, geoid_separation_m};
pub use diff::{SemanticDiff, TileChangeKind, TileDiffEntry};
pub use error::{BuildError, VerifyError};
pub use package::canonical_package_bytes;
pub use provenance::{
    BuildProvenance, Disposition, ParamSnapshot, RecordDisposition, RecordKey, RecordLineage,
    SourceSummary, StageRecord, TOOL_ID, TOOL_VERSION, TileLineage,
};
pub use report::{BuildReports, CoverageReport, HoleCheck, QualityReport, SeamCheck, VoidNode};
pub use source::{
    Aerodrome, LicenseCode, Obstacle, ObstacleKind, Runway, SourceDataset, SourceId, SourceMeta,
    SourceRecordRef, TerrainGrid,
};
pub use verify::{DecodedReports, decode_package_reports, verify_artifact};
