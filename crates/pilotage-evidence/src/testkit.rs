//! Shared fixtures for the crate's unit tests.
//!
//! [`VALID_SLICE`] is a compact but structurally complete ATT-01-shaped graph
//! that passes the gate. The unit tests derive negative cases from it by
//! removing individual lines, so each test isolates the one defect it asserts.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::graph::Graph;
use crate::parse::parse_graph;

/// A complete, gate-passing evidence graph in the canonical text form.
pub(crate) const VALID_SLICE: &str = "\
evidence-graph 1
scope ATT-01
scope-title extreme-attitude behavior traceability slice
scope-requirement AIR-ENV-002
scope-requirement AIR-HAZ-012

node PFD intended-function
title Primary flight display

node FC-ATT-06 hazard
title Polarity / singularity ambiguity

node AIR-ENV-002 safety-requirement
title Attitude envelope determinism

node AIR-HAZ-012 derived-requirement
title Polarity and ordering integrity
attr safety-impact catastrophic-if-credited
attr rationale sign-and-order-preserved-through-singularities
attr disposition notified-and-retained

node DESIGN-SO3 design
title SO(3)-safe attitude presentation

node IMPL-STEP implementation
title UnusualAttitudeState::step

node CASE-BAND verification-case
title sky/ground band matches the f64 reference
locator crates/pilotage-instrument-raster/src/raster/tests/attitude_behavior.rs
attr test sky_ground_band_matches_the_independent_reference

node RESULT-BAND verification-result
title recorded band outcome
attr command cargo test -p pilotage-instrument-raster
attr config-digest 8363e9e81d6654506e95051bec48ea32df50ef10
attr tool-version rustc 1.95.0
attr source-digest git-blob:7ab0d7f2dafef691899dde6f837e3f8561554ec6
attr artifact tests/fixtures/att01-run-output.txt
attr output-digest sha256:d34a8abb658c037032e33e444813d8b935af8f8598e87cd5700b6c5e02f7548b
attr run-id local:8363e9e81d6654506e95051bec48ea32df50ef10
attr outcome pass

node CFG-BASE configuration-item
title worktree baseline

node TOOL-CARGO tool
title cargo test

node REVIEW-1 review
title intended-function review record
attr status complete
attr independent yes
attr reviewer J. Doe
attr date 2026-07-14
attr disposition APPROVED

edge FC-ATT-06 derives-from PFD
edge AIR-ENV-002 mitigates FC-ATT-06
edge AIR-HAZ-012 mitigates FC-ATT-06
edge AIR-ENV-002 allocated-to DESIGN-SO3
edge AIR-HAZ-012 allocated-to DESIGN-SO3
edge DESIGN-SO3 implemented-by IMPL-STEP
edge AIR-ENV-002 verified-by CASE-BAND
edge AIR-HAZ-012 verified-by CASE-BAND
edge CASE-BAND covers AIR-ENV-002
edge CASE-BAND covers AIR-HAZ-012
edge RESULT-BAND result-of CASE-BAND
edge RESULT-BAND covers AIR-ENV-002
edge RESULT-BAND justified-by CFG-BASE
edge RESULT-BAND justified-by TOOL-CARGO
edge REVIEW-1 reviews AIR-HAZ-012
";

/// Parses [`VALID_SLICE`].
pub(crate) fn valid_graph() -> Graph {
    parse_graph(VALID_SLICE).expect("VALID_SLICE parses")
}

/// [`VALID_SLICE`] with every line containing `needle` removed.
pub(crate) fn without(needle: &str) -> String {
    VALID_SLICE
        .lines()
        .filter(|line| !line.contains(needle))
        .collect::<Vec<_>>()
        .join("\n")
}
