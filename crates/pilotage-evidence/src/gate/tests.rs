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
fn a_review_wired_without_reviews_edges_prevents_valid() {
    // The review is attached to the graph through `covers` instead of
    // `reviews`: only `reviews` edges enforce completion, so this wiring
    // would let an explicitly pending review coexist with VALID.
    let text = VALID_SLICE
        .replace(
            "edge REVIEW-1 reviews AIR-HAZ-012",
            "edge REVIEW-1 covers AIR-HAZ-012",
        )
        .replace("attr status complete", "attr status pending");
    assert!(has_open(&text, FindingCode::ReviewIncomplete));
    assert_eq!(verdict_of(&text), GateVerdict::Invalid);
}

#[test]
fn an_orphan_review_slot_prevents_valid() {
    // A review node with no edges at all is a decorative slot: it claims a
    // review exists without ever naming what it reviews.
    let text = without("edge REVIEW-1 reviews AIR-HAZ-012");
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

#[test]
fn a_selector_binds_to_a_genuine_test_definition_only() {
    // A real #[test] fn resolves; every textual decoy is refused: the name in
    // a line comment, in a block comment, in a string literal, a plain helper
    // with no test attribute, and an attribute separated by other code.
    let text = r#"
#[test]
fn real_test() {}

#[tokio::test]
async fn async_real_test() {}

#[test_case(1)]
fn parameterized_test() {}

// fn commented_out(
/* fn block_commented( */
fn helper_only() {
    let _ = "fn string_decoy(";
}

#[test]
fn other() {}
fn after_code_no_attr() {}
"#;
    assert!(super::selector::defines(text, "real_test"));
    assert!(super::selector::defines(text, "async_real_test"));
    assert!(super::selector::defines(text, "parameterized_test"));
    assert!(!super::selector::defines(text, "commented_out"));
    assert!(!super::selector::defines(text, "block_commented"));
    assert!(!super::selector::defines(text, "string_decoy"));
    assert!(!super::selector::defines(text, "helper_only"));
    assert!(!super::selector::defines(text, "after_code_no_attr"));
    assert!(!super::selector::defines(text, "absent_entirely"));
}

/// A JS test fixture exercising the runner rule: one genuine registered
/// test plus every shape that must NOT resolve.
const JS_SELECTOR_FIXTURE: &str = r#"
function testRealBehaviour() {
  assert.equal(1, 1);
}

// Defined at top level but never placed in the runner array.
function testHelperOnly() {}

// Defined and only ever calls itself: recursion is not registration.
function testSelfReferencing() {
  if (false) testSelfReferencing();
}

// Referenced only by another helper's call, never by the runner.
function testCalledByHelper() {}
function driver() {
  testCalledByHelper();
  for (const t of [testInsideHelperArray]) t();
}

// A definition nested inside another function is not a top-level test.
function outer() {
  function testNested() {}
  return testNested;
}

// Decoys in each string flavour and in comments.
// function testLineCommentDecoy() {}
/* function testBlockCommentDecoy() {} */
const dq = "function testDoubleQuoteDecoy(";
const sq = 'function testSingleQuoteDecoy(';
const tpl = `function testTemplateDecoy( ${cooked}`;

function testSubstringBase() {}

for (const test of [
  testRealBehaviour,
  testSubstringBaseExtra,
]) {
  test();
}
"#;

#[test]
fn a_js_selector_needs_a_top_level_definition_and_runner_registration() {
    let js = super::selector::defines_js;
    // The one genuine, registered, top-level test resolves.
    assert!(js(JS_SELECTOR_FIXTURE, "testRealBehaviour"));
    // Structural negatives: definition or registration missing at top level.
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testHelperOnly"),
        "defined but never registered"
    );
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testSelfReferencing"),
        "self-reference is not registration"
    );
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testCalledByHelper"),
        "a helper's call is not registration"
    );
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testInsideHelperArray"),
        "a helper's own for-of is not the runner"
    );
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testNested"),
        "a nested definition is not a top-level test"
    );
    assert!(
        !js(JS_SELECTOR_FIXTURE, "testSubstringBase"),
        "substring of a registered name"
    );
    assert!(!js(JS_SELECTOR_FIXTURE, "testAbsentEntirely"));

    // The extension dispatch routes .mjs/.js to the JS rule and
    // everything else to the Rust rule.
    assert!(super::selector::defines_in(
        "clients/web/x.test.mjs",
        JS_SELECTOR_FIXTURE,
        "testRealBehaviour"
    ));
    assert!(!super::selector::defines_in(
        "src/x.rs",
        JS_SELECTOR_FIXTURE,
        "testRealBehaviour"
    ));
}

#[test]
fn a_js_selector_refuses_comment_and_string_decoys() {
    let js = super::selector::defines_js;
    for decoy in [
        "testLineCommentDecoy",
        "testBlockCommentDecoy",
        "testDoubleQuoteDecoy",
        "testSingleQuoteDecoy",
        "testTemplateDecoy",
    ] {
        assert!(
            !js(JS_SELECTOR_FIXTURE, decoy),
            "a decoy in a comment or string literal must not resolve: {decoy}"
        );
    }
}
