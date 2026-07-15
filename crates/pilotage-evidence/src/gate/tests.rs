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

#[test]
fn a_case_without_a_result_fails() {
    // A second case covers the requirement but records no result. The
    // requirement still reaches the result layer through CASE-BAND, so this is a
    // missing result, not a missing layer.
    let text = format!(
        "{VALID_SLICE}\nnode CASE-EXTRA verification-case\n\
         locator tests/fixtures/sample_tests.rs\nattr test vertical_never_flips\n\
         edge AIR-ENV-002 verified-by CASE-EXTRA\nedge CASE-EXTRA covers AIR-ENV-002\n"
    );
    assert!(has_open(&text, FindingCode::MissingResult));
    assert!(!has_open(&text, FindingCode::MissingDownstreamLayer));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn an_expired_exception_cannot_suppress() {
    // The exception is complete and independently reviewed, but expired: with an
    // as-of date past its expiry it cannot apply, so the gap stays open.
    let gap = without("justified-by CFG-BASE");
    let text = format!(
        "{gap}\n\nexception EX-1\ncovers RESULT-BAND\nowner sokoly\n\
         rationale baseline pending\nstatus open\nexpiry 2020-01-01\nreview REVIEW-1\n"
    );
    let graph = parse_graph(&text).expect("fixture parses");
    let policy = Policy {
        exception_as_of: Some("2026-07-14".to_string()),
        ..Policy::engineering_trace()
    };
    let report = validate(&graph, &policy);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == FindingCode::ResultUnresolved && !f.excepted),
        "the expired exception must not suppress the gap"
    );
    let malformed = report
        .findings
        .iter()
        .find(|f| f.code == FindingCode::ExceptionMalformed)
        .expect("the expired exception is reported malformed");
    assert!(malformed.detail.contains("expired"), "{}", malformed.detail);
    assert_eq!(report.verdict, GateVerdict::Invalid);
}

#[test]
fn a_pending_review_prevents_valid() {
    let text = VALID_SLICE.replace("attr status complete", "attr status pending");
    assert!(has_open(&text, FindingCode::ReviewIncomplete));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn a_non_independent_review_prevents_valid() {
    let text = without("attr independent yes");
    assert!(has_open(&text, FindingCode::ReviewIncomplete));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn a_placeholder_result_fails() {
    // Stripping the executed command leaves the result without full provenance.
    let text = without("attr command");
    assert!(has_open(&text, FindingCode::PlaceholderResult));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn a_source_blob_output_is_a_placeholder() {
    // A result whose execution-output digest is only the test-source blob is a
    // placeholder: it proves what code exists, not that it ran or what it emitted.
    let text = VALID_SLICE.replace(
        "attr output-digest sha256:68f3f7ef347c988e36d958289927da97247fe728b73fd6d2d32adc34aa29f6c2",
        "attr output-digest git-blob:7ab0d7f2dafef691899dde6f837e3f8561554ec6",
    );
    assert!(has_open(&text, FindingCode::PlaceholderResult));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn a_review_without_a_closed_disposition_is_incomplete() {
    // The status still says complete, but the record's disposition is stripped:
    // a completed status with no recorded outcome is not a substantive review.
    let text = without("disposition APPROVED");
    assert!(has_open(&text, FindingCode::ReviewIncomplete));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}
