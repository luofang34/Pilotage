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
use pilotage_evidence::trace::resolve;
use pilotage_evidence::{Graph, NodeId, NodeKind};

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
fn the_real_att01_slice_is_structurally_traced_but_review_pending() {
    let graph = real_graph();
    let pending = report(&graph);
    assert!(
        pending.node_count > 20,
        "the slice is a real graph, not a stub"
    );
    // The trace is structurally complete, but the review is still pending, so the
    // honest verdict is not VALID and the ONLY open finding is the incomplete
    // review — the gate must not green-wash a pending review.
    let open: Vec<FindingCode> = pending
        .findings
        .iter()
        .filter(|f| !f.excepted)
        .map(|f| f.code)
        .collect();
    assert_eq!(
        open,
        vec![FindingCode::ReviewIncomplete],
        "unexpected open findings: {:?}",
        pending.findings
    );
    assert_ne!(pending.verdict, GateVerdict::Valid);

    // Completing the review is all that stands between the slice and VALID:
    // everything else already traces, resolves, and carries result provenance.
    let text = read(&repo_root().join("docs/instruments/evidence-graph.evg"))
        .replace("attr status pending", "attr status complete");
    let completed = parse_graph(&text).expect("edited graph parses");
    let completed_report = report(&completed);
    assert_eq!(
        completed_report.verdict,
        GateVerdict::Valid,
        "open findings once reviewed: {:?}",
        completed_report.findings
    );
}

#[test]
fn the_att01_slice_resolves_bidirectionally() {
    let graph = real_graph();
    let resolution = resolve(&graph);
    assert!(
        resolution.resolves_both_ways(&graph),
        "the ATT-01 slice must resolve forward and backward"
    );

    // Forward: behavior -> hazard -> requirement -> design -> implementation ->
    // verification case -> result.
    for kind in [
        NodeKind::Hazard,
        NodeKind::SafetyRequirement,
        NodeKind::DerivedRequirement,
        NodeKind::Design,
        NodeKind::Implementation,
        NodeKind::VerificationCase,
        NodeKind::VerificationResult,
    ] {
        assert!(
            resolution.forward_reaches(&graph, kind),
            "forward resolution missing {kind:?}"
        );
    }

    // Backward: result -> case -> covered requirement -> design/implementation
    // -> behavior, plus the configuration baseline and tool identity.
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
            "backward resolution missing {kind:?}"
        );
    }

    // The undirected impact view agrees: a result connects back to its
    // requirement and configuration baseline.
    let from_result = impact(&graph, &id("RESULT-RASTER"));
    assert!(from_result.requirements.contains(&id("AIR-ENV-002")));
    assert!(from_result.bundles.contains(&id("CFG-WORKTREE")));
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
fn missing_result_fixture_fails() {
    let report = report(&fixture("missing-result.evg"));
    assert!(has_open(&report, FindingCode::MissingResult));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn pending_review_fixture_fails() {
    let report = report(&fixture("pending-review.evg"));
    assert!(has_open(&report, FindingCode::ReviewIncomplete));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn placeholder_result_fixture_fails() {
    let report = report(&fixture("placeholder-result.evg"));
    assert!(has_open(&report, FindingCode::PlaceholderResult));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn invalid_exception_fixture_fails_when_expired() {
    let graph = fixture("invalid-exception.evg");
    // Past the expiry the reviewed, complete exception can no longer apply, so
    // the gap it covers stays open and is reported.
    let after = Policy {
        exception_as_of: Some("2026-07-14".to_string()),
        ..Policy::engineering_trace()
    };
    let report = validate(&graph, &after);
    assert!(
        has_open(&report, FindingCode::ResultUnresolved),
        "an expired exception must not suppress the gap: {:?}",
        report.findings
    );
    let malformed = report
        .findings
        .iter()
        .find(|f| f.code == FindingCode::ExceptionMalformed)
        .expect("the expired exception is reported");
    assert!(
        malformed.detail.contains("expired"),
        "detail: {}",
        malformed.detail
    );
    assert_eq!(report.verdict, GateVerdict::Invalid);

    // Before its expiry the same exception legitimately applies — proving the
    // as-of date, not a constant, drives the outcome.
    let before = Policy {
        exception_as_of: Some("2019-01-01".to_string()),
        ..Policy::engineering_trace()
    };
    assert_eq!(
        validate(&graph, &before).verdict,
        GateVerdict::ValidWithExceptions
    );
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
