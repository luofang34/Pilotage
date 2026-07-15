//! Stable node identifiers.
//!
//! An identifier is a short, opaque token reused verbatim from the maintained
//! engineering record (`FC-ATT-06`, `AIR-ENV-002`, `PFD`). Keeping the charset
//! narrow makes the identifier a single whitespace-delimited token in the
//! canonical text form and keeps ordering purely lexical, so serialization is
//! deterministic without a normalization pass.

use core::fmt;

use thiserror::Error;

/// Reasons an identifier string is rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    /// The identifier was empty.
    #[error("node identifier is empty")]
    Empty,
    /// The identifier held a character outside `[A-Za-z0-9._:-]`.
    #[error("node identifier {id:?} holds unsupported character {ch:?}")]
    BadChar {
        /// The offending identifier.
        id: String,
        /// The first unsupported character.
        ch: char,
    },
}

/// A stable, content-independent node identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// Validates and wraps an identifier string.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when the string is empty or holds a character
    /// outside `[A-Za-z0-9._:-]`.
    pub fn new(raw: impl Into<String>) -> Result<Self, IdError> {
        let raw = raw.into();
        if raw.is_empty() {
            return Err(IdError::Empty);
        }
        if let Some(ch) = raw.chars().find(|c| !Self::is_allowed(*c)) {
            return Err(IdError::BadChar { id: raw, ch });
        }
        Ok(Self(raw))
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_allowed(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-')
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn accepts_real_identifiers() {
        for id in [
            "FC-ATT-06",
            "AIR-ENV-002",
            "PFD",
            "TOOL:cargo-test",
            "a.b_c",
        ] {
            assert!(NodeId::new(id).is_ok(), "{id} should be accepted");
        }
    }

    #[test]
    fn rejects_empty_and_whitespace() {
        assert_eq!(NodeId::new(""), Err(IdError::Empty));
        assert!(matches!(
            NodeId::new("has space"),
            Err(IdError::BadChar { ch: ' ', .. })
        ));
    }

    #[test]
    fn ordering_is_lexical() {
        let mut ids = [
            NodeId::new("b").unwrap(),
            NodeId::new("a").unwrap(),
            NodeId::new("a.1").unwrap(),
        ];
        ids.sort();
        assert_eq!(ids[0].as_str(), "a");
    }
}
