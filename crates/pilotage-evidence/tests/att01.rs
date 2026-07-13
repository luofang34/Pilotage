//! Integration tests for the ATT-01 vertical slice and the negative fixtures.
//!
//! These use the public API only. The real maintained record lives at
//! `docs/instruments/evidence-graph.evg`; the negative fixtures live beside this
//! file under `tests/fixtures/`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pilotage_evidence::canonical::to_canonical;
use pilotage_evidence::gate::{FindingCode, GateReport, GateVerdict, validate, validate_resolving};
use pilotage_evidence::impact::impact;
use pilotage_evidence::parse::parse_graph;
use pilotage_evidence::policy::Policy;
use pilotage_evidence::{Graph, NodeId};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn real_graph() -> Graph {
    let path = repo_root().join("docs/instruments/evidence-graph.evg");
    parse_graph(&read(&path)).expect("the maintained ATT-01 graph parses")
}

fn fixture(name: &str) -> Graph {
    let path = crate_dir().join("tests/fixtures").join(name);
    parse_graph(&read(&path)).unwrap_or_else(|e| panic!("fixture {name} parses: {e}"))
}

fn report(graph: &Graph) -> GateReport {
    validate(graph, &Policy::engineering_trace())
}

fn has_open(report: &GateReport, code: FindingCode) -> bool {
    report
        .findings
        .iter()
        .any(|f| f.code == code && !f.excepted)
}

fn id(raw: &str) -> NodeId {
    NodeId::new(raw).expect("valid id")
}

#[test]
fn the_real_att01_slice_is_structurally_valid() {
    let report = report(&real_graph());
    assert_eq!(
        report.verdict,
        GateVerdict::Valid,
        "open findings: {:?}",
        report.findings
    );
    assert!(
        report.node_count > 20,
        "the slice is a real graph, not a stub"
    );
}

#[test]
fn the_att01_slice_resolves_bidirectionally() {
    let graph = real_graph();
    // behavior -> current result
    let from_requirement = impact(&graph, &id("AIR-ENV-002"));
    assert!(
        from_requirement.tests.contains(&id("RESULT-RASTER")),
        "AIR-ENV-002 must resolve forward to a recorded result"
    );
    // result -> behavior / configuration
    let from_result = impact(&graph, &id("RESULT-RASTER"));
    assert!(
        from_result.requirements.contains(&id("AIR-ENV-002")),
        "a result must resolve back to its requirement"
    );
    assert!(
        from_result.bundles.contains(&id("CFG-WORKTREE")),
        "a result must resolve back to its configuration baseline"
    );
}

#[test]
fn the_real_slice_canonical_form_round_trips() {
    let graph = real_graph();
    let once = to_canonical(&graph);
    let twice = to_canonical(&parse_graph(&once).expect("canonical re-parses"));
    assert_eq!(once, twice);
}

#[test]
fn empty_fixture_fails_closed() {
    let report = report(&fixture("empty.evg"));
    assert!(has_open(&report, FindingCode::EmptyGraph));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn missing_root_fixture_fails() {
    let report = report(&fixture("missing-root.evg"));
    assert!(has_open(&report, FindingCode::MissingScopeRequirement));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn missing_layer_fixture_fails() {
    let report = report(&fixture("missing-layer.evg"));
    assert!(has_open(&report, FindingCode::MissingDownstreamLayer));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn dangling_edge_fixture_fails() {
    let report = report(&fixture("dangling-edge.evg"));
    assert!(has_open(&report, FindingCode::DanglingEdge));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn unresolved_selector_fixture_fails() {
    let report = report(&fixture("unresolved-selector.evg"));
    assert!(has_open(&report, FindingCode::UnresolvedSelector));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn orphan_requirement_fixture_fails() {
    let report = report(&fixture("orphan-requirement.evg"));
    assert!(has_open(&report, FindingCode::MissingUpstream));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn a_present_selector_resolves_on_disk() {
    let graph = fixture("attitude-slice.evg");
    let report = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert_eq!(
        report.verdict,
        GateVerdict::Valid,
        "open findings: {:?}",
        report.findings
    );
}

#[test]
fn a_selector_naming_a_missing_symbol_fails_resolution() {
    let graph = fixture("unresolvable-selector-fs.evg");
    // Structurally it is complete; only filesystem resolution catches the gap.
    assert_eq!(report(&graph).verdict, GateVerdict::Valid);
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert!(has_open(&resolved, FindingCode::UnresolvedSelector));
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
}
