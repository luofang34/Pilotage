//! Integration tests for the ATT-01 vertical slice and the negative fixtures.
//!
//! These use the public API only. The real maintained record lives at
//! `docs/instruments/evidence-graph.evg`; the negative fixtures live beside this
//! file under `tests/fixtures/`. The execution-output artifact negatives
//! (digest, run-id, and path-containment) live in `artifact_resolution.rs`.

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
fn the_real_att01_slice_is_traced_and_its_recorded_review_is_valid() {
    let graph = real_graph();
    let current = report(&graph);
    assert!(
        current.node_count > 20,
        "the slice is a real graph, not a stub"
    );
    // The trace is structurally complete and the independent review is
    // recorded (rec-att-01), so the slice is VALID with no open findings.
    assert_eq!(
        current.verdict,
        GateVerdict::Valid,
        "open findings: {:?}",
        current.findings
    );

    let raw = read(&repo_root().join("docs/instruments/evidence-graph.evg"));

    // Green-washing guard: a completed status with the substantive record
    // fields stripped is incomplete again — the status string alone is never
    // enough.
    let stripped = parse_graph(&raw.replace(
        "attr status complete\nattr independent yes\nattr reviewer sokoly\n\
         attr date 2026-07-15\nattr disposition approved",
        "attr status complete\nattr independent yes",
    ))
    .expect("edited graph parses");
    assert!(has_open(&report(&stripped), FindingCode::ReviewIncomplete));
    assert_ne!(report(&stripped).verdict, GateVerdict::Valid);

    // And a review whose status regresses to pending is never valid, however
    // complete its recorded fields remain.
    let pending = parse_graph(&raw.replace("attr status complete", "attr status pending"))
        .expect("edited graph parses");
    assert!(has_open(&report(&pending), FindingCode::ReviewIncomplete));
    assert_ne!(report(&pending).verdict, GateVerdict::Valid);
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
fn unexecuted_result_fixture_fails() {
    // The result's execution-output digest is a source blob, not a captured run.
    let report = report(&fixture("unexecuted-result.evg"));
    assert!(has_open(&report, FindingCode::PlaceholderResult));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn unbacked_review_fixture_fails() {
    // Status says complete, but the substantive record disposition is PENDING.
    let report = report(&fixture("unbacked-review.evg"));
    assert!(has_open(&report, FindingCode::ReviewIncomplete));
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn the_att01_artifacts_parse_and_match_the_results() {
    // Happy path: every ATT-01 result's committed artifact resolves, hashes to
    // its output-digest, its parsed command/config/tool/run-id/outcome match
    // the result node, and every source-digest matches the current test
    // sources — so the fully resolved slice is VALID with no open findings.
    let resolved = validate_resolving(&real_graph(), &Policy::engineering_trace(), &repo_root());
    assert!(
        !has_open(&resolved, FindingCode::PlaceholderResult),
        "an ATT-01 artifact failed to resolve or match: {:?}",
        resolved.findings
    );
    let open: Vec<FindingCode> = resolved
        .findings
        .iter()
        .filter(|f| !f.excepted)
        .map(|f| f.code)
        .collect();
    assert_eq!(open, vec![], "findings: {:?}", resolved.findings);
    assert_eq!(resolved.verdict, GateVerdict::Valid);
}

#[test]
fn unresolved_review_entry_fixture_fails_on_the_specific_entry() {
    // The record file holds a complete entry AND the pending entry this review
    // names; resolution must fail on the NAMED (pending) entry, not the file.
    let graph = fixture("unresolved-review-entry.evg");
    assert_eq!(report(&graph).verdict, GateVerdict::Valid);
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert!(
        has_open(&resolved, FindingCode::ReviewIncomplete),
        "findings: {:?}",
        resolved.findings
    );
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
}

#[test]
fn a_review_naming_a_complete_entry_resolves() {
    // Pointed at the file's complete entry, the same review passes — proving
    // specific-entry resolution accepts a genuine record, not just rejects one.
    let raw = read(&crate_dir().join("tests/fixtures/unresolved-review-entry.evg"));
    let graph = parse_graph(&raw.replace("#rec-pending", "#rec-complete")).expect("parses");
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert_eq!(
        resolved.verdict,
        GateVerdict::Valid,
        "findings: {:?}",
        resolved.findings
    );
}

#[test]
fn a_review_status_cannot_outrun_its_named_record_entry() {
    // The real review node stays fully populated, but its locator is pointed
    // at an anchor the record file does not contain: entry resolution must
    // reject the claim — a completed status is only as good as the specific,
    // resolvable record entry it names.
    let raw = read(&repo_root().join("docs/instruments/evidence-graph.evg"));
    let text = raw.replace("#rec-att-01", "#rec-does-not-exist");
    let graph = parse_graph(&text).expect("edited graph parses");
    // Structurally the node still looks complete...
    assert_eq!(report(&graph).verdict, GateVerdict::Valid);
    // ...but the named entry does not resolve, so resolution fails closed.
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &repo_root());
    assert!(
        has_open(&resolved, FindingCode::ReviewIncomplete),
        "findings: {:?}",
        resolved.findings
    );
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
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

#[test]
fn a_selector_naming_a_non_test_function_fails_resolution() {
    // The symbol exists textually in the file, but it is a plain helper with
    // no #[test] attribute: the harness would never run it, so a result
    // "recorded" for it is not evidence and the selector must not resolve.
    let graph = fixture("selector-not-a-test.evg");
    assert_eq!(report(&graph).verdict, GateVerdict::Valid);
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert!(
        has_open(&resolved, FindingCode::UnresolvedSelector),
        "findings: {:?}",
        resolved.findings
    );
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
}
