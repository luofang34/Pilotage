//! The declared assurance scope and its explicit exceptions.
//!
//! The scope names which requirements the no-orphan gate holds to a full
//! upstream-and-downstream trace. An [`Exception`] is the only sanctioned way a
//! known gap is allowed to stand: it is an explicit record, it is always
//! surfaced in the report, and — by policy — it may need an independent review.
//! It can never turn an unexplained gap into a silent success.

use std::collections::BTreeSet;

use crate::id::NodeId;

/// The declared in-scope assurance case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The scope identifier (e.g. `ATT-01`).
    pub id: String,
    /// A short human-readable title.
    pub title: String,
    /// The requirements held to the full no-orphan trace.
    pub requirements: BTreeSet<NodeId>,
}

impl Scope {
    /// A named, titled scope with no requirements yet.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            requirements: BTreeSet::new(),
        }
    }

    /// Whether the scope declares any in-scope requirement.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }
}

/// An explicit, justified exception to the no-orphan policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exception {
    /// The exception's own identifier.
    pub id: String,
    /// The node whose gap this exception covers.
    pub covers: NodeId,
    /// The accountable owner.
    pub owner: String,
    /// Why the gap is accepted.
    pub rationale: String,
    /// Lifecycle status (e.g. `open`, `accepted`).
    pub status: String,
    /// When the exception lapses and must be revisited.
    pub expiry: String,
    /// The independent review node, when one is recorded.
    pub review: Option<NodeId>,
}

impl Exception {
    /// The mandatory free-text fields that are present but empty, if any.
    ///
    /// An exception missing any of these cannot suppress a finding; the gate
    /// reports the exception itself as malformed instead.
    #[must_use]
    pub fn missing_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.owner.trim().is_empty() {
            missing.push("owner");
        }
        if self.rationale.trim().is_empty() {
            missing.push("rationale");
        }
        if self.status.trim().is_empty() {
            missing.push("status");
        }
        if self.expiry.trim().is_empty() {
            missing.push("expiry");
        }
        missing
    }
}
