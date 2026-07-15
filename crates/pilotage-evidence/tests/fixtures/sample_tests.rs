//! Selector-resolution target for the pilotage-evidence integration tests.
//!
//! This file is never compiled: it lives under `tests/fixtures/` so cargo does
//! not treat it as a test crate. The gate reads it as text to confirm a
//! verification-case selector names a real `#[test]` function — and the decoys
//! below confirm a selector can NOT resolve against a commented-out test, a
//! name inside a string literal, or a plain helper the harness would not run.

#[test]
fn band_matches_reference() {}

#[test]
fn vertical_never_flips() {}

// A commented-out test must never satisfy a selector:
// #[test]
// fn commented_decoy() {}

fn helper_not_a_test() {
    let _ = "fn string_decoy() { this is prose inside a string literal }";
}
