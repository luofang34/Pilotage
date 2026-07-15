//! Negative fixtures for execution-output artifact resolution: an artifact
//! that is absent, forged, recorded against another run, unstructured, or
//! reached through an escaping path must each fail the gate on resolution.
//! Every fixture here is structurally VALID so resolution alone decides.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use pilotage_evidence::Graph;
use pilotage_evidence::gate::{
    Finding, FindingCode, GateReport, GateVerdict, validate, validate_resolving,
};
use pilotage_evidence::parse::parse_graph;
use pilotage_evidence::policy::Policy;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> Graph {
    let path = crate_dir().join("tests/fixtures").join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_graph(&text).unwrap_or_else(|e| panic!("fixture {name} parses: {e}"))
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

/// Resolves a fixture against the crate root and returns the first open
/// placeholder-result finding — the shared shape of every artifact negative.
fn resolved_placeholder(name: &str) -> Finding {
    let graph = fixture(name);
    assert_eq!(
        report(&graph).verdict,
        GateVerdict::Valid,
        "{name} must be structurally valid so resolution alone decides"
    );
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
    resolved
        .findings
        .into_iter()
        .find(|f| f.code == FindingCode::PlaceholderResult && !f.excepted)
        .unwrap_or_else(|| panic!("{name}: no placeholder-result finding"))
}

#[test]
fn unresolvable_artifact_fixture_fails() {
    // The output-digest names an artifact that is not committed: structurally
    // valid, but its digest cannot be bound to any content on resolution.
    let graph = fixture("unresolvable-artifact.evg");
    assert_eq!(report(&graph).verdict, GateVerdict::Valid);
    let resolved = validate_resolving(&graph, &Policy::engineering_trace(), &crate_dir());
    assert!(
        has_open(&resolved, FindingCode::PlaceholderResult),
        "findings: {:?}",
        resolved.findings
    );
    assert_eq!(resolved.verdict, GateVerdict::Invalid);
}

#[test]
fn mismatched_artifact_fixture_fails() {
    // The artifact is committed, but its content does not hash to the declared
    // output-digest — a detached/forged digest, caught on resolution.
    let finding = resolved_placeholder("mismatched-artifact.evg");
    assert!(
        finding.detail.contains("hashes to"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn field_mismatch_artifact_fixture_fails() {
    // The artifact hashes to the declared digest, but its parsed command names a
    // different package than the result node declares.
    let finding = resolved_placeholder("field-mismatch-artifact.evg");
    assert!(
        finding.detail.contains("command"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn source_as_artifact_fixture_fails() {
    // A test source file used as the execution-output artifact hashes correctly
    // but is not a structured run record, so it carries none of the run fields.
    let finding = resolved_placeholder("source-as-artifact.evg");
    assert!(
        finding.detail.contains("not a structured run record"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn runid_mismatch_artifact_fixture_fails() {
    // The artifact hashes to the declared digest, but its parsed run-id names a
    // different run than the result node declares — hash equality alone must
    // not bind evidence recorded against some other run.
    let finding = resolved_placeholder("runid-mismatch-artifact.evg");
    assert!(
        finding.detail.contains("run-id"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn absolute_artifact_path_fixture_fails() {
    // An absolute artifact path is refused before any filesystem read.
    let finding = resolved_placeholder("absolute-artifact.evg");
    assert!(
        finding.detail.contains("absolute"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn traversal_artifact_path_fixture_fails() {
    // A parent ('..') component is refused: the declared path may not climb
    // out of the evidence root, even toward a file that exists.
    let finding = resolved_placeholder("traversal-artifact.evg");
    assert!(
        finding.detail.contains("parent"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn stale_source_artifact_fixture_fails() {
    // The artifact resolves and every run-record field matches, but the
    // declared source-digest names an older blob of the test source: the
    // recorded result predates the current test and must not keep passing.
    let finding = resolved_placeholder("stale-source-artifact.evg");
    assert!(
        finding.detail.contains("source-digest")
            && finding.detail.contains("predates the test source"),
        "detail: {}",
        finding.detail
    );
}

#[test]
fn symlink_escape_artifact_fixture_fails() {
    // The declared path is relative and clean, but it is a committed symlink
    // whose canonical target lies outside the evidence root.
    let finding = resolved_placeholder("symlink-escape-artifact.evg");
    assert!(
        finding.detail.contains("outside the evidence root"),
        "detail: {}",
        finding.detail
    );
}
