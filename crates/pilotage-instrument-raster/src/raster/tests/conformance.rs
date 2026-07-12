//! Backend-conformance corpus: the reference half.
//!
//! This module owns the reviewed scene corpus and its reference outcomes, and
//! emits the shared golden the browser interpreter replays
//! (`clients/web/scene-conformance-corpus.json`). The tests here are the
//! reference-side wall:
//!
//! - the golden is exactly the reference's canonical serialization (drift
//!   guard: CI compares, it never rewrites — regeneration is a reviewed,
//!   env-gated action);
//! - every typed failure class the gate and decoder can raise appears in the
//!   corpus;
//! - the resource budgets flip verdict exactly at their limits;
//! - the fail-safe paint faults spoil the reference frame.
//!
//! The browser side is `clients/web/scene-conformance.test.mjs`, which pins
//! itself to the same golden and to its `corpusSha256`.

#![allow(clippy::expect_used, clippy::panic)]

mod corpus;
mod manifest;
mod outcomes;

use std::path::PathBuf;

use pilotage_instrument_scene::validate_layers;

use self::corpus::{CorpusEntry, corpus};
use self::manifest::{corpus_sha256, manifest_json};
use crate::{FrameId, FramebufferDims, RasterError, render};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../clients/web/scene-conformance-corpus.json")
}

fn entry<'a>(all: &'a [CorpusEntry], name: &str) -> &'a CorpusEntry {
    all.iter()
        .find(|e| e.name == name)
        .expect("named corpus entry exists")
}

fn render_bytes(bytes: &[u8]) -> Result<(), RasterError> {
    let mut fb = std::vec![0u8; 64 * 64 * 4];
    render(
        bytes,
        &mut fb,
        FramebufferDims::tight(64, 64),
        FrameId::default(),
    )
    .map(|_| ())
}

/// Regenerates the golden. Gated behind `REGEN_CONFORMANCE_CORPUS` so CI never
/// rewrites it: a reviewer runs this deliberately, inspects the diff, and bumps
/// the corpus version/reason in `manifest.rs` before committing.
#[test]
fn regenerate_golden_when_requested() {
    if std::env::var_os("REGEN_CONFORMANCE_CORPUS").is_none() {
        return;
    }
    let json = manifest_json(&corpus());
    std::fs::write(golden_path(), json).expect("write golden");
}

#[test]
fn golden_matches_reference() {
    let path = golden_path();
    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Regenerate with REGEN_CONFORMANCE_CORPUS=1 cargo test -p pilotage-instrument-raster",
            path.display()
        )
    });
    let regenerated = manifest_json(&corpus());
    assert_eq!(
        on_disk, regenerated,
        "golden corpus drifted from the reference. Review the change, bump the corpus version/reason, and regenerate with REGEN_CONFORMANCE_CORPUS=1."
    );
}

#[test]
fn corpus_hash_is_pinned_in_the_golden() {
    let hash = corpus_sha256(&corpus());
    assert_eq!(hash.len(), 64, "sha-256 is 64 hex chars");
    let golden = std::fs::read_to_string(golden_path()).expect("golden present");
    assert!(
        golden.contains(&hash),
        "the golden must carry the current corpus hash"
    );
}

#[test]
fn corpus_covers_every_typed_failure_class() {
    let all = corpus();
    let mut classes = std::collections::BTreeSet::new();
    for e in &all {
        let o = outcomes::outcome_of(e);
        if let Some(c) = o.gate_error {
            classes.insert(c);
        }
        if let Some(c) = o.decode_error {
            classes.insert(c);
        }
    }
    for expected in [
        "DuplicateLayer",
        "OutOfOrder",
        "NestedLayer",
        "EndWithoutBegin",
        "EndMismatch",
        "UnclosedLayer",
        "CommandOutsideLayer",
        "UnisolatedState",
        "UnbalancedState",
        "StackOverCapacity",
        "OverCapacity",
        "SceneTooLarge",
        "Decode:BadVersion",
        "Decode:Truncated",
        "Decode:BadPayload",
        "BadVersion",
        "Truncated",
        "BadPayload",
    ] {
        assert!(
            classes.contains(expected),
            "corpus is missing failure class {expected}"
        );
    }
}

#[test]
fn resource_budgets_flip_verdict_at_their_limits() {
    let all = corpus();
    assert!(validate_layers(&entry(&all, "stack-depth-at-limit").bytes).is_ok());
    assert!(matches!(
        validate_layers(&entry(&all, "stack-depth-over-limit").bytes),
        Err(pilotage_instrument_scene::LayerError::StackOverCapacity { .. })
    ));
    assert!(validate_layers(&entry(&all, "layer-commands-at-limit").bytes).is_ok());
    assert!(matches!(
        validate_layers(&entry(&all, "layer-commands-over-limit").bytes),
        Err(pilotage_instrument_scene::LayerError::OverCapacity { .. })
    ));
    assert!(validate_layers(&entry(&all, "scene-bytes-at-limit").bytes).is_ok());
    assert!(matches!(
        validate_layers(&entry(&all, "scene-bytes-over-limit").bytes),
        Err(pilotage_instrument_scene::LayerError::SceneTooLarge { .. })
    ));
}

#[test]
fn paint_faults_spoil_the_reference_frame() {
    let all = corpus();
    assert!(matches!(
        render_bytes(&entry(&all, "paint-non-finite").bytes),
        Err(RasterError::NonFinite)
    ));
    assert!(matches!(
        render_bytes(&entry(&all, "paint-out-of-range").bytes),
        Err(RasterError::CoordinateOutOfRange { .. })
    ));
    assert!(matches!(
        render_bytes(&entry(&all, "paint-too-many-vertices").bytes),
        Err(RasterError::TooManyVertices { .. })
    ));
    assert!(matches!(
        render_bytes(&entry(&all, "text-uncovered").bytes),
        Err(RasterError::Glyph(_))
    ));
    assert!(render_bytes(&entry(&all, "text-covered").bytes).is_ok());
}

#[test]
fn framebuffer_geometry_budgets_are_enforced() {
    let all = corpus();
    let scene = &entry(&all, "empty-background-canonical").bytes;
    let mut fb = std::vec![0u8; 64 * 64 * 4];
    let go = |dims, fb: &mut [u8]| render(scene, fb, dims, FrameId::default());
    assert!(matches!(
        go(FramebufferDims::tight(0, 4), &mut fb),
        Err(RasterError::ZeroFramebuffer)
    ));
    assert!(matches!(
        go(FramebufferDims::tight(crate::MAX_DIMENSION + 1, 1), &mut fb),
        Err(RasterError::FramebufferTooLarge { .. })
    ));
    let narrow = FramebufferDims {
        width: 8,
        height: 8,
        stride_bytes: 8,
    };
    assert!(matches!(
        go(narrow, &mut fb),
        Err(RasterError::StrideTooSmall { .. })
    ));
    let mut tiny = std::vec![0u8; 16];
    assert!(matches!(
        go(FramebufferDims::tight(8, 8), &mut tiny),
        Err(RasterError::FramebufferTooSmall { .. })
    ));
}
