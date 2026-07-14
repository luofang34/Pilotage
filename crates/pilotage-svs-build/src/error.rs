//! Typed, fail-closed failure model for the processing chain.
//!
//! Every [`BuildError`] variant aborts the build and emits no package: a chain
//! that cannot convert a datum, cannot honor its parameters, or produces output
//! its own verifier rejects refuses to sign anything rather than emit a partial
//! or plausible-looking database. There is no repair path and no partial
//! artifact — a stage failure is a refusal, so a tool fault can never activate a
//! package.

use pilotage_geo::GeoError;
use pilotage_svs_db::DbError;

/// Why the processing chain refused to produce a package. Each variant carries
/// the context its message needs; the chain returns it and emits nothing.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// A build parameter is outside its admissible range, so the chain cannot
    /// run deterministically.
    #[error("invalid build configuration: {reason}")]
    InvalidConfig {
        /// What was wrong with the configuration.
        reason: &'static str,
    },
    /// A source record declares a datum this build cannot interpret. An unknown
    /// datum is refused rather than guessed, so no height or coordinate is
    /// silently mis-referenced.
    #[error("source {source_id} declares an unknown {axis} datum; refused rather than guessed")]
    UnknownSourceDatum {
        /// The source the offending record came from.
        source_id: u32,
        /// Which axis (`horizontal` or `vertical`) was unknown.
        axis: &'static str,
    },
    /// A source datum needs a declared identity (a realization for NAD83/ITRF, a
    /// geoid for geometric MSL) that the source did not supply.
    #[error("source {source_id} datum is under-declared: {reason}")]
    UndeclaredSourceIdentity {
        /// The source the offending record came from.
        source_id: u32,
        /// What declaration was missing.
        reason: &'static str,
    },
    /// The requested datum conversion is not one this build implements. A
    /// conversion it does not know is refused rather than approximated.
    #[error("unsupported {axis} datum conversion from code {from} to code {to}")]
    UnsupportedDatumConversion {
        /// Source datum wire code.
        from: u8,
        /// Target datum wire code.
        to: u8,
        /// Which axis the conversion was on.
        axis: &'static str,
    },
    /// A source coordinate or height was not finite, so it has no place in a
    /// deterministic grid.
    #[error("source {source_id} carries a non-finite {field}")]
    NonFiniteInput {
        /// The source the offending record came from.
        source_id: u32,
        /// Which field was non-finite.
        field: &'static str,
    },
    /// A source terrain grid is not usable (non-positive step, mismatched post
    /// count, or non-finite origin).
    #[error("source {source_id} terrain grid is invalid: {reason}")]
    InvalidTerrainGrid {
        /// The source the grid came from.
        source_id: u32,
        /// What was wrong with the grid.
        reason: &'static str,
    },
    /// A geodetic value could not be constructed from the (post-conversion)
    /// coordinates, so the record cannot be tiled.
    #[error("source {source_id} produced an invalid geodetic position")]
    InvalidGeodetic {
        /// The source the offending record came from.
        source_id: u32,
        /// The underlying geospatial error.
        #[source]
        source: GeoError,
    },
    /// After processing, no tile survived, so signing would emit an empty
    /// package. Refused rather than activating a database that covers nothing.
    #[error("processing produced no tiles; refusing to emit an empty package")]
    EmptyOutput,
    /// The chain's own output failed the SVS-02 verifier. This is an internal
    /// inconsistency, not an input fault; the chain refuses to hand back a
    /// package that would not activate.
    #[error("built package failed self-verification against the SVS-02 verifier: {source}")]
    SelfVerification {
        /// The verifier's refusal.
        #[source]
        source: DbError,
    },
    /// The provenance/report bundle could not be serialized for signing, so the
    /// build cannot bind and sign it. Refused rather than emitting an unsigned
    /// bundle.
    #[error("provenance/report bundle serialization failed: {source}")]
    BundleSerialization {
        /// The serialization error.
        #[source]
        source: serde_json::Error,
    },
}

/// Why an emitted artifact failed independent verification. Each variant is a
/// refusal: a package that does not verify, a bundle whose signature does not
/// verify or is not bound to the package, or reports that disagree with the
/// package they claim to describe.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The package itself failed SVS-02 verification.
    #[error("package failed SVS-02 verification: {source}")]
    Package {
        /// The verifier's refusal.
        #[source]
        source: DbError,
    },
    /// The provenance/report bundle could not be serialized to re-derive the
    /// bytes the signature covers.
    #[error("bundle serialization failed: {source}")]
    BundleSerialization {
        /// The serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// The bundle signature did not verify against the trust root — the
    /// provenance or reports were altered, the signer is untrusted, or the key
    /// is malformed.
    #[error("bundle signature failed verification; provenance or reports were altered")]
    BundleSignatureInvalid,
    /// The bundle names a different package than the one it travels with, so it
    /// is not bound to this package.
    #[error("bundle is not bound to this package: provenance content hash does not match")]
    BundleBindingMismatch,
    /// A report field disagrees with the value re-derived by decoding the
    /// produced package.
    #[error("report field {field} disagrees with the decoded package")]
    ReportMismatch {
        /// The field that disagreed.
        field: &'static str,
    },
    /// A tile payload could not be decoded, so the package cannot be
    /// independently re-derived.
    #[error("package tile payload could not be decoded: {reason}")]
    PayloadDecode {
        /// What was wrong with the payload.
        reason: &'static str,
    },
    /// A decoded package record has no matching lineage entry, so an output
    /// record is untraceable.
    #[error(
        "decoded package record (class {class}, tile {lat_index},{lon_index}) has no lineage entry"
    )]
    LineageMissingForRecord {
        /// The record's feature-class wire code.
        class: u8,
        /// Tile latitude index.
        lat_index: i32,
        /// Tile longitude index.
        lon_index: i32,
    },
    /// A lineage entry has no matching decoded package record, so it describes a
    /// record the package does not contain.
    #[error(
        "lineage entry (class {class}, tile {lat_index},{lon_index}) has no decoded package record"
    )]
    LineageOrphan {
        /// The record's feature-class wire code.
        class: u8,
        /// Tile latitude index.
        lat_index: i32,
        /// Tile longitude index.
        lon_index: i32,
    },
    /// Two records share one identity on the package or lineage side, so the
    /// mapping cannot be a bijection.
    #[error("duplicate record identity (class {class}) breaks the record-lineage bijection")]
    DuplicateRecord {
        /// The record's feature-class wire code.
        class: u8,
    },
    /// A recorded source content digest does not match the source input, so the
    /// provenance does not describe the source it claims.
    #[error("source {source_id} content digest does not match the source input")]
    SourceDigestMismatch {
        /// The source whose digest disagreed.
        source_id: u32,
    },
    /// A lineage record lists no source, so an output record traces to nothing.
    #[error("lineage record (class {class}, tile {lat_index},{lon_index}) lists no source")]
    EmptyLineageSources {
        /// The record's feature-class wire code.
        class: u8,
        /// Tile latitude index.
        lat_index: i32,
        /// Tile longitude index.
        lon_index: i32,
    },
    /// A lineage source reference names a source with no signed summary, so it is
    /// dangling.
    #[error("lineage references source {source_id}, which has no signed source summary")]
    UnknownLineageSource {
        /// The referenced source with no summary.
        source_id: u32,
    },
    /// A lineage record lists the same source reference twice.
    #[error("lineage record references source {source_id} record {record} more than once")]
    DuplicateLineageSource {
        /// The duplicated source.
        source_id: u32,
        /// The duplicated record index.
        record: u32,
    },
    /// A lineage source reference names a record index beyond the source's
    /// recorded record count.
    #[error("lineage references source {source_id} record {record}, out of range (count {count})")]
    SourceRecordOutOfRange {
        /// The referenced source.
        source_id: u32,
        /// The out-of-range record index.
        record: u32,
        /// The source's recorded record count.
        count: u32,
    },
    /// A signed source summary is referenced by no lineage record, so it is an
    /// extra source the output does not draw from.
    #[error("source summary {source_id} is referenced by no lineage record")]
    UnreferencedSourceSummary {
        /// The unreferenced source.
        source_id: u32,
    },
    /// A dataset source has no signed summary, so the provenance does not cover
    /// every source.
    #[error("dataset source {source_id} has no signed source summary")]
    SourceSummaryMissing {
        /// The dataset source with no summary.
        source_id: u32,
    },
}
