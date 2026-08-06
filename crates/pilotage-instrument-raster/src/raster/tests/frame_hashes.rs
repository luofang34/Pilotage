#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::{BUILTIN_PANELS, PANEL_H, PANEL_W, PfdConfig, draw_hsi, draw_pfd};
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::{AircraftState, FreshnessPolicy, resolve};
use sha2::{Digest, Sha256};
use std::vec::Vec;

use crate::{FrameId, FramebufferDims, RenderStatus, render};

// Frame hashes pinned from a byte-reproducible render on the reference
// rasterizer, owned by each panel's descriptor (`raster_baseline`) so a
// panel travels with its regression baseline. `libm` plus IEEE-754 f32
// make these identical across the supported CI architectures; a
// mismatch is a determinism regression, not a value to re-pin
// casually. The PFD hash covers the datum-qualified altitude tape: the
// fixture's local-relative reference paints the amber REL label and
// the not-applied SET setting box (ALT-01). The HSI hash covers the
// reference-typed heading: the rose turns with the fixture's explicit
// SIM-declared independent sample — never quaternion yaw — and paints
// the amber SIM reference label (NAV-01).
fn pinned_baseline(id: &str) -> &'static str {
    BUILTIN_PANELS
        .iter()
        .find(|panel| panel.id == id)
        .and_then(|panel| panel.raster_baseline)
        .expect("every builtin panel carries a raster baseline")
}

/// The shared canonical "typical" state (ADR-0033): the same fixture
/// the scene digest draws, so the pinned frame hashes and the digest
/// exercise one corpus.
pub(super) fn demo_state() -> AircraftState {
    pilotage_instrument_registry::states::typical()
}

pub(super) fn encode(build: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut w = SceneWriter::new(&mut buf).expect("writer");
    build(&mut w);
    let n = w.finish();
    buf.truncate(n);
    buf
}

fn frame(scene: &[u8]) -> Vec<u8> {
    let (w, h) = (PANEL_W as u32, PANEL_H as u32);
    let mut fb = std::vec![0u8; (w * h * 4) as usize];
    let report = render(
        scene,
        &mut fb,
        FramebufferDims::tight(w, h),
        FrameId::default(),
    )
    .expect("panel scene renders");
    assert_eq!(report.status, RenderStatus::Painted);
    fb
}

fn sha_hex(bytes: &[u8]) -> std::string::String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = std::string::String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[test]
fn pfd_frame_hash_is_reproducible_and_pinned() {
    let data = resolve(&demo_state(), &FreshnessPolicy::default());
    let scene = encode(|w| draw_pfd(&data, &PfdConfig::default(), None, w).expect("pfd"));
    let first = frame(&scene);
    let second = frame(&scene);
    assert_eq!(
        first, second,
        "PFD frame is bit-reproducible across renders"
    );
    assert_eq!(sha_hex(&first), pinned_baseline("pfd"));
}

#[test]
fn hsi_frame_hash_is_reproducible_and_pinned() {
    let data = resolve(&demo_state(), &FreshnessPolicy::default());
    let scene = encode(|w| draw_hsi(&data, None, w).expect("hsi"));
    let first = frame(&scene);
    let second = frame(&scene);
    assert_eq!(
        first, second,
        "HSI frame is bit-reproducible across renders"
    );
    assert_eq!(sha_hex(&first), pinned_baseline("hsi"));
}

/// A shape gate, not a value check: the per-panel hash tests above own
/// value correctness; this keeps a new builtin from shipping with no
/// baseline at all (Registry::new does not require one).
#[test]
fn all_builtin_panels_carry_a_baseline() {
    for panel in BUILTIN_PANELS {
        let baseline = panel
            .raster_baseline
            .unwrap_or_else(|| panic!("{} ships without a raster baseline", panel.id));
        assert_eq!(
            baseline.len(),
            64,
            "{} baseline is not sha256 hex",
            panel.id
        );
        assert!(baseline.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

#[test]
fn monitor_frame_hash_is_reproducible_and_pinned() {
    let data = resolve(&demo_state(), &FreshnessPolicy::default());
    let scene =
        encode(|w| pilotage_instrument_panels::draw_monitor(&data, None, w).expect("monitor"));
    let first = frame(&scene);
    let second = frame(&scene);
    assert_eq!(
        first, second,
        "monitor frame is bit-reproducible across renders"
    );
    assert_eq!(sha_hex(&first), pinned_baseline("monitor"));
}
