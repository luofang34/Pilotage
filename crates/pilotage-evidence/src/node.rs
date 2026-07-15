//! Typed lifecycle nodes.
//!
//! The kinds cover the whole engineering lifecycle without committing to any
//! one standard's vocabulary. A node records what it *is* (its [`NodeKind`]),
//! where the real artifact lives (`locator`), and any policy-relevant
//! attributes (safety impact, rationale, disposition, a test selector). It does
//! not encode standard-objective mappings; those are views held elsewhere.

use std::collections::BTreeMap;

use crate::id::NodeId;

/// The kind of lifecycle artifact a node stands for.
///
/// Different kinds have different permitted roots and leaves; the gate does not
/// force a single `SYS -> HLR -> LLR -> TEST` spine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    /// An intended function of the product (e.g. the PFD).
    IntendedFunction,
    /// A failure condition / hazard from the hazard assessment.
    Hazard,
    /// A safety goal or safety-relevant requirement.
    SafetyRequirement,
    /// A requirement derived during design rather than flowed from a parent.
    DerivedRequirement,
    /// An architecture or detailed-design element.
    Design,
    /// An implementation unit (a function, type, or module).
    Implementation,
    /// A verification case (a named, executable check).
    VerificationCase,
    /// A verification procedure (how a case is run).
    VerificationProcedure,
    /// A recorded verification result / outcome.
    VerificationResult,
    /// A coverage or completeness analysis.
    CoverageAnalysis,
    /// A review of another node.
    Review,
    /// An approval / sign-off of another node.
    Approval,
    /// A configuration item / baseline.
    ConfigurationItem,
    /// A tool relied upon to produce evidence.
    Tool,
    /// A reported anomaly / problem report.
    Anomaly,
    /// Externally supplied evidence.
    ExternalEvidence,
}

impl NodeKind {
    /// Every kind, in a fixed order (used by tests and help text).
    pub const ALL: [NodeKind; 16] = [
        NodeKind::IntendedFunction,
        NodeKind::Hazard,
        NodeKind::SafetyRequirement,
        NodeKind::DerivedRequirement,
        NodeKind::Design,
        NodeKind::Implementation,
        NodeKind::VerificationCase,
        NodeKind::VerificationProcedure,
        NodeKind::VerificationResult,
        NodeKind::CoverageAnalysis,
        NodeKind::Review,
        NodeKind::Approval,
        NodeKind::ConfigurationItem,
        NodeKind::Tool,
        NodeKind::Anomaly,
        NodeKind::ExternalEvidence,
    ];

    /// The stable token used in the canonical text form.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            NodeKind::IntendedFunction => "intended-function",
            NodeKind::Hazard => "hazard",
            NodeKind::SafetyRequirement => "safety-requirement",
            NodeKind::DerivedRequirement => "derived-requirement",
            NodeKind::Design => "design",
            NodeKind::Implementation => "implementation",
            NodeKind::VerificationCase => "verification-case",
            NodeKind::VerificationProcedure => "verification-procedure",
            NodeKind::VerificationResult => "verification-result",
            NodeKind::CoverageAnalysis => "coverage-analysis",
            NodeKind::Review => "review",
            NodeKind::Approval => "approval",
            NodeKind::ConfigurationItem => "configuration-item",
            NodeKind::Tool => "tool",
            NodeKind::Anomaly => "anomaly",
            NodeKind::ExternalEvidence => "external-evidence",
        }
    }

    /// Parses a token back to a kind.
    #[must_use]
    pub fn from_token(token: &str) -> Option<NodeKind> {
        NodeKind::ALL.into_iter().find(|k| k.token() == token)
    }

    /// Whether this kind is a requirement the scope may declare in-scope.
    #[must_use]
    pub fn is_requirement(self) -> bool {
        matches!(
            self,
            NodeKind::SafetyRequirement | NodeKind::DerivedRequirement
        )
    }
}

/// A typed lifecycle node with a stable identity and free-form attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The node's stable identifier.
    pub id: NodeId,
    /// What the node is.
    pub kind: NodeKind,
    /// A short human-readable title.
    pub title: String,
    /// Where the real artifact lives (a path, anchor, or `path::symbol`).
    pub locator: Option<String>,
    /// Policy-relevant attributes, keyed for deterministic ordering.
    pub attrs: BTreeMap<String, String>,
}

impl Node {
    /// A node with the given identity and kind and no attributes.
    #[must_use]
    pub fn new(id: NodeId, kind: NodeKind, title: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            title: title.into(),
            locator: None,
            attrs: BTreeMap::new(),
        }
    }

    /// The value of an attribute, if present and non-empty.
    #[must_use]
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}
