//! Change-impact analysis over the evidence graph.
//!
//! Given a changed node, [`impact`] returns everything connected to it — the
//! requirements, coverage analyses, verification artifacts, reviews, and
//! configuration bundles that a change might invalidate. It walks every relation
//! in both directions, so the answer is deliberately conservative: it over-lists
//! rather than miss an affected artifact.

use crate::graph::Graph;
use crate::id::NodeId;
use crate::node::NodeKind;
use crate::relation::RelationKind;

/// The artifacts a change to one node may affect, bucketed by kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpactReport {
    /// The node whose change was analysed.
    pub changed: NodeId,
    /// Whether that node exists in the graph.
    pub found: bool,
    /// Affected requirements (safety and derived).
    pub requirements: Vec<NodeId>,
    /// Affected coverage analyses.
    pub analyses: Vec<NodeId>,
    /// Affected verification cases, procedures, and results.
    pub tests: Vec<NodeId>,
    /// Affected reviews and approvals.
    pub reviews: Vec<NodeId>,
    /// Affected configuration items (baselines / bundles).
    pub bundles: Vec<NodeId>,
    /// Other affected nodes (functions, hazards, design, implementation,
    /// tools, anomalies, external evidence).
    pub other: Vec<NodeId>,
}

/// Computes the change impact of `changed`.
#[must_use]
pub fn impact(graph: &Graph, changed: &NodeId) -> ImpactReport {
    let all = RelationKind::ALL.to_vec();
    let reached = graph.reachable_via(changed, &all, &all);
    let mut report = ImpactReport {
        changed: changed.clone(),
        found: graph.contains(changed),
        requirements: Vec::new(),
        analyses: Vec::new(),
        tests: Vec::new(),
        reviews: Vec::new(),
        bundles: Vec::new(),
        other: Vec::new(),
    };
    for id in reached {
        match graph.kind_of(&id) {
            Some(NodeKind::SafetyRequirement | NodeKind::DerivedRequirement) => {
                report.requirements.push(id);
            }
            Some(NodeKind::CoverageAnalysis) => report.analyses.push(id),
            Some(
                NodeKind::VerificationCase
                | NodeKind::VerificationProcedure
                | NodeKind::VerificationResult,
            ) => report.tests.push(id),
            Some(NodeKind::Review | NodeKind::Approval) => report.reviews.push(id),
            Some(NodeKind::ConfigurationItem) => report.bundles.push(id),
            Some(_) => report.other.push(id),
            None => {}
        }
    }
    report
}
