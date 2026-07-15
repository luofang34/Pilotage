//! The crate's top-level error type.

use thiserror::Error;

use crate::parse::ParseError;

/// A failure loading or parsing an evidence graph.
///
/// Validation (the gate) does not error: an invalid graph is a *report*, not an
/// `Err`. This type covers only the I/O and parse steps that precede it.
#[derive(Debug, Error)]
pub enum EvidenceError {
    /// The graph file could not be read.
    #[error("reading evidence graph {path:?}")]
    Read {
        /// The path that could not be read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The graph text did not parse.
    #[error("parsing evidence graph")]
    Parse(#[from] ParseError),
}
