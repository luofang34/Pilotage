#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn priority_order_is_warning_highest() {
    assert!(AlertClass::Warning > AlertClass::Caution);
    assert!(AlertClass::Caution > AlertClass::Advisory);
    assert!(AlertClass::Advisory > AlertClass::Status);
    assert!(AlertClass::Status > AlertClass::Maintenance);
}

#[test]
fn aural_tokens_match_classes() {
    assert_eq!(
        AlertClass::Warning.aural_token(),
        AuralToken::ContinuousTone
    );
    assert_eq!(AlertClass::Caution.aural_token(), AuralToken::TripleChime);
    assert_eq!(AlertClass::Advisory.aural_token(), AuralToken::SingleChime);
    assert_eq!(AlertClass::Status.aural_token(), AuralToken::Silent);
    assert_eq!(AlertClass::Maintenance.aural_token(), AuralToken::Silent);
}

#[test]
fn only_warning_is_continuous() {
    assert!(AuralToken::ContinuousTone.is_continuous());
    assert!(!AuralToken::TripleChime.is_continuous());
    assert!(!AuralToken::SingleChime.is_continuous());
    assert!(!AuralToken::Silent.is_continuous());
}

#[test]
fn warning_and_caution_latch() {
    assert!(AlertClass::Warning.latches());
    assert!(AlertClass::Caution.latches());
    assert!(!AlertClass::Advisory.latches());
    assert!(!AlertClass::Status.latches());
    assert!(!AlertClass::Maintenance.latches());
}

#[test]
fn declutter_retains_warning_and_caution() {
    assert!(!AlertClass::Warning.declutters_under_unusual());
    assert!(!AlertClass::Caution.declutters_under_unusual());
    assert!(AlertClass::Advisory.declutters_under_unusual());
    assert!(AlertClass::Status.declutters_under_unusual());
    assert!(AlertClass::Maintenance.declutters_under_unusual());
}
