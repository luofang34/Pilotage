//! Selector-resolution target for the pilotage-evidence integration tests.
//!
//! This file is never compiled: it lives under `tests/fixtures/` so cargo does
//! not treat it as a test crate. The gate reads it as text to confirm a
//! verification-case selector names a real `fn`.

fn band_matches_reference() {}

fn vertical_never_flips() {}
