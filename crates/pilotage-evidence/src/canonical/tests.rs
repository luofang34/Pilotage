//! Unit tests for canonical serialization and content digests.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::canonical::{graph_digest, node_digest, to_canonical};
use crate::id::NodeId;
use crate::parse::parse_graph;
use crate::testkit::{VALID_SLICE, valid_graph};

fn pfd_digest(text: &str) -> [u8; 32] {
    let graph = parse_graph(text).expect("parses");
    let node = graph
        .node(&NodeId::new("PFD").unwrap())
        .expect("PFD present");
    node_digest(node)
}

#[test]
fn canonical_form_is_a_fixed_point() {
    let once = to_canonical(&valid_graph());
    let twice = to_canonical(&parse_graph(&once).expect("canonical re-parses"));
    assert_eq!(
        once, twice,
        "serialize -> parse -> serialize must be byte-identical"
    );
}

#[test]
fn canonical_form_is_independent_of_authoring_order() {
    let ordered = "evidence-graph 1\nscope S\nscope-requirement R1\n\
        node R1 safety-requirement\nnode R2 safety-requirement\nedge R1 decomposes-to R2\n";
    let shuffled = "evidence-graph 1\nscope S\n\
        node R2 safety-requirement\nnode R1 safety-requirement\n\
        scope-requirement R1\nedge R1 decomposes-to R2\n";
    let a = to_canonical(&parse_graph(ordered).expect("a parses"));
    let b = to_canonical(&parse_graph(shuffled).expect("b parses"));
    assert_eq!(a, b);
}

#[test]
fn a_node_digest_is_stable_and_content_sensitive() {
    assert_eq!(
        pfd_digest(VALID_SLICE),
        pfd_digest(VALID_SLICE),
        "stable across parses"
    );
    let retitled = VALID_SLICE.replace("Primary flight display", "Something else entirely");
    assert_ne!(
        pfd_digest(VALID_SLICE),
        pfd_digest(&retitled),
        "a changed title must change the node digest"
    );
}

#[test]
fn the_graph_digest_tracks_content() {
    let base = graph_digest(&valid_graph());
    let changed = VALID_SLICE.replace("UnusualAttitudeState::step", "something_else");
    let changed = graph_digest(&parse_graph(&changed).expect("parses"));
    assert_ne!(base, changed);
}

#[test]
fn an_unrelated_edit_leaves_a_node_digest_alone() {
    // Retitling IMPL-STEP must not disturb PFD's digest.
    let edited = VALID_SLICE.replace("UnusualAttitudeState::step", "renamed_symbol");
    assert_eq!(pfd_digest(VALID_SLICE), pfd_digest(&edited));
}
