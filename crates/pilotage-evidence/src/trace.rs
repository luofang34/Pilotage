//! Bidirectional resolution of the declared scope slice.
//!
//! Forward resolution walks from the intended-function roots outward toward the
//! recorded results: behavior -> hazard -> requirement -> design ->
//! implementation -> verification case -> result -> configuration / tool.
//! Backward resolution walks from each recorded result back to behavior:
//! result -> case -> covered requirement -> design / implementation -> hazard ->
//! intended function, together with the configuration baseline and tool identity
//! that justify the result.
//!
//! Both walks reuse the single [`Graph::reachable_via`] primitive with fixed
//! relation configurations, so "the slice resolves both ways" is a property of
//! the recorded edges, not a hard-coded `SYS -> HLR -> LLR -> TEST` spine.

use std::collections::BTreeSet;

use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::NodeKind;
use crate::relation::RelationKind;

/// The nodes reached resolving the scope forward and backward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The declared scope id.
    pub scope_id: String,
    /// Nodes reached walking forward from the intended-function roots.
    pub forward: BTreeSet<NodeId>,
    /// Nodes reached walking backward from the recorded results.
    pub backward: BTreeSet<NodeId>,
}

impl Resolution {
    /// Whether the forward walk reaches at least one node of `kind`.
    #[must_use]
    pub fn forward_reaches(&self, graph: &Graph, kind: NodeKind) -> bool {
        self.forward
            .iter()
            .any(|id| graph.kind_of(id) == Some(kind))
    }

    /// Whether the backward walk reaches at least one node of `kind`.
    #[must_use]
    pub fn backward_reaches(&self, graph: &Graph, kind: NodeKind) -> bool {
        self.backward
            .iter()
            .any(|id| graph.kind_of(id) == Some(kind))
    }

    /// Whether the slice resolves end to end in both directions: forward from
    /// behavior down to a recorded result, and backward from a result up to
    /// behavior and to the configuration baseline and tool identity that justify
    /// it.
    #[must_use]
    pub fn resolves_both_ways(&self, graph: &Graph) -> bool {
        self.forward_reaches(graph, NodeKind::VerificationResult)
            && self.backward_reaches(graph, NodeKind::IntendedFunction)
            && self.backward_reaches(graph, NodeKind::ConfigurationItem)
            && self.backward_reaches(graph, NodeKind::Tool)
    }
}

/// Relations walked forward (behavior -> result).
const FORWARD_FWD: [RelationKind; 4] = [
    RelationKind::AllocatedTo,
    RelationKind::ImplementedBy,
    RelationKind::VerifiedBy,
    RelationKind::JustifiedBy,
];

/// Relations walked against their direction while resolving forward.
const FORWARD_REV: [RelationKind; 3] = [
    RelationKind::DerivesFrom,
    RelationKind::Mitigates,
    RelationKind::ResultOf,
];

/// Relations walked forward (result -> behavior, design, implementation).
const BACKWARD_FWD: [RelationKind; 7] = [
    RelationKind::ResultOf,
    RelationKind::Covers,
    RelationKind::Mitigates,
    RelationKind::DerivesFrom,
    RelationKind::AllocatedTo,
    RelationKind::ImplementedBy,
    RelationKind::JustifiedBy,
];

/// Relations walked against their direction while resolving backward.
const BACKWARD_REV: [RelationKind; 1] = [RelationKind::VerifiedBy];

/// Resolves the declared scope forward from behavior and backward from results.
#[must_use]
pub fn resolve(graph: &Graph) -> Resolution {
    Resolution {
        scope_id: graph.scope.id.clone(),
        forward: walk(
            graph,
            NodeKind::IntendedFunction,
            &FORWARD_FWD,
            &FORWARD_REV,
        ),
        backward: walk(
            graph,
            NodeKind::VerificationResult,
            &BACKWARD_FWD,
            &BACKWARD_REV,
        ),
    }
}

/// The union of every node reachable from a root of `root_kind`, roots included.
fn walk(
    graph: &Graph,
    root_kind: NodeKind,
    forward: &[RelationKind],
    reverse: &[RelationKind],
) -> BTreeSet<NodeId> {
    let mut reached = BTreeSet::new();
    for root in graph.ids_of_kind(root_kind) {
        reached.insert(root.clone());
        reached.extend(graph.reachable_via(root, forward, reverse));
    }
    reached
}

#[cfg(test)]
mod tests;
