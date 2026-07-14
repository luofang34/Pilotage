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
}
