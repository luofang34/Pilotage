#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::{PANEL_H, PANEL_W, PfdConfig, draw_hsi, draw_pfd};
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::{AircraftState, FreshnessPolicy, resolve};
use sha2::{Digest, Sha256};
use std::vec::Vec;

use crate::{FrameId, FramebufferDims, RenderStatus, render};

// Frame hashes pinned from a byte-reproducible render on the reference
// rasterizer. `libm` plus IEEE-754 f32 make these identical across the
// supported CI architectures; a mismatch is a determinism regression, not a
// value to re-pin casually. The PFD hash covers the datum-qualified
// altitude tape: the fixture's local-relative reference paints the amber
// REL label and the not-applied SET setting box (ALT-01). The HSI hash
// covers the reference-typed heading: the rose turns with the fixture's
// explicit SIM-declared independent sample — never quaternion yaw — and
// paints the amber SIM reference label (NAV-01).
const PFD_SHA256: &str = "43b49bde6bbf7372d704d54214d4a3d0b9cd3ad09e86862a8ffc20fd6ae05ef1";
const HSI_SHA256: &str = "66653ce135e6f2163fa48d805a0ab1a8f3d0ac51d778f7b1eb2aa4ec05bfbb7c";

/// The shared canonical "typical" state (ADR-0033): the same fixture
/// the scene digest draws, so the pinned frame hashes and the digest
/// exercise one corpus. The unchanged hash values below prove the
/// shared copy renders bit-identically to the fixture this module used
/// to own.
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
    assert_eq!(sha_hex(&first), PFD_SHA256);
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
    assert_eq!(sha_hex(&first), HSI_SHA256);
}
