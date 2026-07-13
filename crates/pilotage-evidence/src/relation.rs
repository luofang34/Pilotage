//! Typed relations between nodes.
//!
//! Every edge is directed `from --relation--> to`. The direction is fixed and
//! documented per relation so the gate can traverse "upstream toward hazards
//! and intended functions" and "downstream toward implementation and results"
//! deterministically.

use crate::id::NodeId;

/// The kind of a directed relation.
///
/// Read each as `from <relation> to`:
/// - `DerivesFrom`: a child requirement/hazard derives from its source.
/// - `Mitigates`: a requirement/design mitigates a hazard.
/// - `AllocatedTo`: a requirement is allocated to a design element.
/// - `Satisfies`: a design/implementation satisfies a requirement.
/// - `ImplementedBy`: a requirement/design is implemented by a unit.
/// - `VerifiedBy`: a requirement/design is verified by a case.
/// - `ResultOf`: a result is the outcome of running a case.
/// - `Covers`: a case/analysis covers a requirement.
/// - `Reviews`: a review reviews another node.
/// - `Approves`: an approval approves another node.
/// - `JustifiedBy`: a node is justified by rationale/external evidence.
/// - `DecomposesTo`: a parent requirement decomposes to a child.
/// - `IndependentFrom`: two nodes are asserted independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationKind {
    /// `from` derives from `to`.
    DerivesFrom,
    /// `from` mitigates hazard `to`.
    Mitigates,
    /// `from` is allocated to design `to`.
    AllocatedTo,
    /// `from` satisfies requirement `to`.
    Satisfies,
    /// `from` is implemented by unit `to`.
    ImplementedBy,
    /// `from` is verified by case `to`.
    VerifiedBy,
    /// `from` is the result of case `to`.
    ResultOf,
    /// `from` covers requirement `to`.
    Covers,
    /// `from` reviews `to`.
    Reviews,
    /// `from` approves `to`.
    Approves,
    /// `from` is justified by `to`.
    JustifiedBy,
    /// `from` decomposes to `to`.
    DecomposesTo,
    /// `from` is independent from `to`.
    IndependentFrom,
}

impl RelationKind {
    /// Every relation, in a fixed order.
    pub const ALL: [RelationKind; 13] = [
        RelationKind::DerivesFrom,
        RelationKind::Mitigates,
        RelationKind::AllocatedTo,
        RelationKind::Satisfies,
        RelationKind::ImplementedBy,
        RelationKind::VerifiedBy,
        RelationKind::ResultOf,
        RelationKind::Covers,
        RelationKind::Reviews,
        RelationKind::Approves,
        RelationKind::JustifiedBy,
        RelationKind::DecomposesTo,
        RelationKind::IndependentFrom,
    ];

    /// The stable token used in the canonical text form.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            RelationKind::DerivesFrom => "derives-from",
            RelationKind::Mitigates => "mitigates",
            RelationKind::AllocatedTo => "allocated-to",
            RelationKind::Satisfies => "satisfies",
            RelationKind::ImplementedBy => "implemented-by",
            RelationKind::VerifiedBy => "verified-by",
            RelationKind::ResultOf => "result-of",
            RelationKind::Covers => "covers",
            RelationKind::Reviews => "reviews",
            RelationKind::Approves => "approves",
            RelationKind::JustifiedBy => "justified-by",
            RelationKind::DecomposesTo => "decomposes-to",
            RelationKind::IndependentFrom => "independent-from",
        }
    }

    /// Parses a token back to a relation.
    #[must_use]
    pub fn from_token(token: &str) -> Option<RelationKind> {
        RelationKind::ALL.into_iter().find(|r| r.token() == token)
    }
}

/// A directed, typed edge between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    /// The source node.
    pub from: NodeId,
    /// The relation kind.
    pub relation: RelationKind,
    /// The target node.
    pub to: NodeId,
}

impl Edge {
    /// A new edge.
    #[must_use]
    pub fn new(from: NodeId, relation: RelationKind, to: NodeId) -> Self {
        Self { from, relation, to }
    }
}
