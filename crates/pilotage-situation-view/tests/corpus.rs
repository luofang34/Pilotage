//! Public conformance-corpus test for the reference composer.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_situation_view::{load_corpus_v1, verify_reference_corpus_v1};

#[test]
fn corpus_contains_five_required_scenarios() {
    let corpus = load_corpus_v1().expect("corpus must decode");
    assert_eq!(corpus.cases.len(), 5);
}

#[test]
fn reference_composer_passes_the_shared_corpus() {
    verify_reference_corpus_v1().expect("reference composer must conform");
}
