//! Unit tests for bidirectional scope resolution.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::node::NodeKind;
use crate::parse::parse_graph;
use crate::testkit::{valid_graph, without};
use crate::trace::resolve;

#[test]
fn the_valid_slice_resolves_in_both_directions() {
    let graph = valid_graph();
    let resolution = resolve(&graph);
    assert!(resolution.resolves_both_ways(&graph));
    // Forward, from behavior, reaches the full downstream chain to a result.
    for kind in [
        NodeKind::Hazard,
        NodeKind::SafetyRequirement,
        NodeKind::Design,
        NodeKind::Implementation,
        NodeKind::VerificationCase,
        NodeKind::VerificationResult,
    ] {
        assert!(
            resolution.forward_reaches(&graph, kind),
            "forward walk missing {kind:?}"
        );
    }
    // Backward, from the result, reaches design, implementation, and behavior.
    for kind in [
        NodeKind::VerificationCase,
        NodeKind::SafetyRequirement,
        NodeKind::Design,
        NodeKind::Implementation,
        NodeKind::Hazard,
        NodeKind::IntendedFunction,
        NodeKind::ConfigurationItem,
        NodeKind::Tool,
    ] {
        assert!(
            resolution.backward_reaches(&graph, kind),
            "backward walk missing {kind:?}"
        );
    }
}

#[test]
fn a_slice_with_no_result_does_not_resolve() {
    let graph = parse_graph(&without("RESULT-BAND")).expect("fixture parses");
    let resolution = resolve(&graph);
    assert!(!resolution.forward_reaches(&graph, NodeKind::VerificationResult));
    assert!(!resolution.resolves_both_ways(&graph));
}
