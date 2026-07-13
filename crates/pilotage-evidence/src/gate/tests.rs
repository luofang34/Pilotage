//! Unit tests for the scoped no-orphan gate: one positive baseline and one
//! negative case per fail-closed rule.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::gate::{FindingCode, GateVerdict, validate};
use crate::parse::parse_graph;
use crate::policy::Policy;
use crate::testkit::{VALID_SLICE, valid_graph, without};

fn verdict_of(text: &str) -> GateVerdict {
    let graph = parse_graph(text).expect("fixture parses");
    validate(&graph, &Policy::engineering_trace()).verdict
}

/// Whether a non-excepted finding of `code` is present.
fn has_open(text: &str, code: FindingCode) -> bool {
    let graph = parse_graph(text).expect("fixture parses");
    validate(&graph, &Policy::engineering_trace())
        .findings
        .iter()
        .any(|f| f.code == code && !f.excepted)
}

#[test]
fn the_att01_slice_passes() {
    let report = validate(&valid_graph(), &Policy::engineering_trace());
    assert_eq!(
        report.verdict,
        GateVerdict::Valid,
        "findings: {:?}",
        report.findings
    );
    assert!(report.findings.is_empty());
}

#[test]
fn empty_graph_fails_closed() {
    let text = "evidence-graph 1\nscope ATT-01\n";
    assert!(has_open(text, FindingCode::EmptyGraph));
    assert_eq!(verdict_of(text), GateVerdict::Invalid);
}

#[test]
fn missing_root_fails() {
    let text = format!("{VALID_SLICE}scope-requirement AIR-GHOST-001\n");
    assert!(has_open(&text, FindingCode::MissingScopeRequirement));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn missing_required_layer_fails() {
    // Dropping the result node leaves the downstream chain a layer short.
    let text = without("RESULT-BAND");
    assert!(has_open(&text, FindingCode::MissingDownstreamLayer));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn dangling_edge_fails() {
    let text = format!("{VALID_SLICE}edge AIR-ENV-002 verified-by GHOST-CASE\n");
    assert!(has_open(&text, FindingCode::DanglingEdge));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn unresolved_selector_fails() {
    let text = without("attr test");
    assert!(has_open(&text, FindingCode::UnresolvedSelector));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn orphan_requirement_upstream_fails() {
    let text = without("mitigates FC-ATT-06");
    assert!(has_open(&text, FindingCode::MissingUpstream));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn result_missing_baseline_fails() {
    let text = without("justified-by CFG-BASE");
    assert!(has_open(&text, FindingCode::ResultUnresolved));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn derived_requirement_missing_disposition_fails() {
    let text = without("attr disposition");
    assert!(has_open(&text, FindingCode::DerivedRequirementIncomplete));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn derived_requirement_without_review_fails() {
    let text = without("REVIEW-1 reviews");
    assert!(has_open(&text, FindingCode::DerivedRequirementIncomplete));
}

#[test]
fn a_well_formed_exception_suppresses_but_is_still_reported() {
    let gap = without("justified-by CFG-BASE");
    let text = format!(
        "{gap}\n\nexception EX-1\ncovers RESULT-BAND\nowner sokoly\n\
         rationale baseline pending\nstatus open\nexpiry 2026-09-01\nreview REVIEW-1\n"
    );
    let graph = parse_graph(&text).expect("fixture parses");
    let report = validate(&graph, &Policy::engineering_trace());
    assert_eq!(report.verdict, GateVerdict::ValidWithExceptions);
    let excepted = report
        .findings
        .iter()
        .find(|f| f.code == FindingCode::ResultUnresolved)
        .expect("the result finding is still present");
    assert!(
        excepted.excepted,
        "the finding must remain visible, only suppressed"
    );
}

#[test]
fn a_malformed_exception_cannot_suppress() {
    // Same gap, but the exception omits its owner: it cannot apply.
    let gap = without("justified-by CFG-BASE");
    let text = format!(
        "{gap}\n\nexception EX-1\ncovers RESULT-BAND\n\
         rationale baseline pending\nstatus open\nexpiry 2026-09-01\nreview REVIEW-1\n"
    );
    assert!(has_open(&text, FindingCode::ResultUnresolved));
    assert!(has_open(&text, FindingCode::ExceptionMalformed));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}
