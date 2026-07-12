#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::{PANEL_H, PANEL_W, PfdConfig, draw_hsi, draw_pfd};
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Quat, Selections, SnapshotMeta, Stamped, ValidFlags, Wind,
};
use pilotage_instrument_state::{FreshnessPolicy, resolve};
use sha2::{Digest, Sha256};
use std::vec::Vec;

use crate::{FrameId, FramebufferDims, RenderStatus, render};

// Frame hashes pinned from a byte-reproducible render on the reference
// rasterizer. `libm` plus IEEE-754 f32 make these identical across the
// supported CI architectures; a mismatch is a determinism regression, not a
// value to re-pin casually. The PFD hash covers the datum-qualified
// altitude tape: the fixture's local-relative reference paints the amber
// REL label and the not-applied SET setting box (ALT-01).
const PFD_SHA256: &str = "3148b8e9d5c3d9cc5c1ac812c3f5674615649c65d4966ad3f1a85d1ce1f1d952";
const HSI_SHA256: &str = "6edcbc92d936a690a68f0632d2ec20b158d54e59cc9fde6640feefa077b258e3";

/// A fixed, richly populated state so every panel band paints content.
///
/// The fixture must resolve bit-identically whether or not fail-safe
/// validation is in the resolution path: trust is declared explicitly
/// (defaults must not be relied on), and the quaternion's squared
/// component sum is exactly 1.0 in f32, so a validating resolver's
/// renormalization divides by exactly 1.0 and changes nothing.
pub(super) fn demo_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat {
                    w: 0.5,
                    x: 0.5,
                    y: 0.5,
                    z: 0.5,
                },
                rates_rps: [0.02, -0.01, 0.05],
            }),
            age_ms: Some(80.0),
        },
        kinematics: Stamped {
            data: Some(Kinematics {
                pos_ned_m: [1200.0, 340.0, -305.0],
                vel_ned_mps: [52.0, 9.0, -2.0],
            }),
            age_ms: Some(80.0),
        },
        air: Stamped {
            data: Some(AirData {
                ias_mps: Some(53.0),
                baro_setting_hpa: Some(1013.2),
            }),
            age_ms: Some(80.0),
        },
        nav: Stamped {
            data: Some(NavData {
                source: NavSource::Gps,
                course_rad: 0.6,
                cdi_dots: 0.7,
                fromto: NavFromTo::To,
                vdev_dots: Some(-0.4),
                dist_nm: Some(12.4),
            }),
            age_ms: Some(80.0),
        },
        wind: Stamped {
            data: Some(Wind {
                from_rad: 2.1,
                speed_mps: 7.5,
            }),
            age_ms: Some(80.0),
        },
        selections: Selections {
            heading_bug_rad: 0.5,
            altitude_sel_m: Some(915.0),
            ..Selections::default()
        },
        quality: EstimateQuality::Good,
        valid: ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
        },
        snapshot: SnapshotMeta::default(),
        altitude: pilotage_instrument_state::AltitudeDeclaration::default(),
    }
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
