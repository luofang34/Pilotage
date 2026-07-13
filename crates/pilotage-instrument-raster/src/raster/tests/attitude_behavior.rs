#![allow(clippy::expect_used, clippy::panic)]
//! ATT-01 raster behavior: the rendered sky/ground band at extreme and
//! inverted attitudes, checked at the pixel level against an independent
//! f64 down-vector reference.
//!
//! The scene tests prove the geometry the panel *emits*; these prove what
//! the reference rasterizer actually *paints*, closing the loop from
//! attitude to lit pixels. The reference here derives display pitch and
//! bank straight from the quaternion in f64 — a code path independent of
//! the f32 `libm` presentation under test — so a sign or rotation error in
//! the draw layer surfaces as a misclassified pixel, not a plausible frame.

use pilotage_instrument_panels::{PANEL_H, PANEL_W, PfdConfig, draw_pfd};
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, EstimateQuality, FreshnessPolicy, Kinematics, Quat, Stamped,
    ValidFlags, resolve,
};
use std::vec::Vec;

use crate::{FrameId, FramebufferDims, RenderStatus, render};

const W: u32 = PANEL_W as u32;
const H: u32 = PANEL_H as u32;
const CX: f64 = PANEL_W as f64 / 2.0;
const CY: f64 = PANEL_H as f64 / 2.0;
/// Horizon geometry constant shared with `pfd/horizon.rs`: screen pixels
/// per degree of display pitch.
const PX_PER_DEG_PITCH: f64 = 7.2;

/// Sky and ground fills (mirrors `panels::palette`), the only two colors a
/// clean background sample can take.
const SKY: [u8; 3] = [0, 110, 210];
const GROUND: [u8; 3] = [140, 96, 44];
const RED: [u8; 3] = [255, 0, 0];

/// f32 ZYX euler → quaternion, matching the panel orientation fixtures.
fn euler_quat(roll_deg: f32, pitch_deg: f32, yaw_deg: f32) -> Quat {
    let d = core::f32::consts::PI / 180.0;
    let (r, p, y) = (roll_deg * d / 2.0, pitch_deg * d / 2.0, yaw_deg * d / 2.0);
    let (cr, sr) = (libm::cosf(r), libm::sinf(r));
    let (cp, sp) = (libm::cosf(p), libm::sinf(p));
    let (cy, sy) = (libm::cosf(y), libm::sinf(y));
    Quat {
        w: cr * cp * cy + sr * sp * sy,
        x: sr * cp * cy - cr * sp * sy,
        y: cr * sp * cy + sr * cp * sy,
        z: cr * cp * sy - sr * sp * cy,
    }
}

/// A valid flying state at the given orientation, populated enough that the
/// PFD paints every band.
fn state_at(roll_deg: f32, pitch_deg: f32) -> AircraftState {
    let mut state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: euler_quat(roll_deg, pitch_deg, 0.0),
                rates_rps: [0.0, 0.0, 0.0],
            }),
            age_ms: Some(20.0),
        },
        ..AircraftState::default()
    };
    state.quality = EstimateQuality::Good;
    state.valid = ValidFlags {
        attitude: true,
        rates: true,
        position: true,
        velocity: true,
        ..ValidFlags::default()
    };
    state.kinematics = Stamped {
        data: Some(Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [40.0, 0.0, -1.0],
        }),
        age_ms: Some(20.0),
    };
    state.air = Stamped {
        data: Some(AirData {
            ias_mps: Some(40.0),
            baro_setting_hpa: Some(1013.0),
        }),
        age_ms: Some(20.0),
    };
    state
}

/// Independent f64 display (pitch_rad, bank_rad) from the quaternion's
/// world-down vector in body coordinates — the same quadratic form used
/// physically, recomputed here in f64 as the oracle. Valid away from the
/// vertical bank-hold window.
fn ref_pitch_bank(q: Quat) -> (f64, f64) {
    let (w, x, y, z) = (
        f64::from(q.w),
        f64::from(q.x),
        f64::from(q.y),
        f64::from(q.z),
    );
    let down_x = 2.0 * (x * z - w * y);
    let down_y = 2.0 * (y * z + w * x);
    let down_z = 1.0 - 2.0 * (x * x + y * y);
    let pitch = -down_x.clamp(-1.0, 1.0).asin();
    let bank = down_y.atan2(down_z);
    (pitch, bank)
}

/// True where the oracle places sky at screen pixel `(px, py)`: transform
/// the screen offset into the roll-rotated attitude frame and compare to
/// the horizon row. Sky is the half above (smaller rotated-y than) the
/// horizon.
fn ref_is_sky(px: u32, py: u32, pitch_rad: f64, bank_rad: f64) -> bool {
    let dx = f64::from(px) + 0.5 - CX;
    let dy = f64::from(py) + 0.5 - CY;
    let y_rot = dx * bank_rad.sin() + dy * bank_rad.cos();
    let horizon_y = pitch_rad.to_degrees() * PX_PER_DEG_PITCH;
    y_rot < horizon_y
}

fn pixel(fb: &[u8], px: u32, py: u32) -> [u8; 3] {
    let i = ((py * W + px) * 4) as usize;
    [fb[i], fb[i + 1], fb[i + 2]]
}

fn manhattan(a: [u8; 3], b: [u8; 3]) -> i32 {
    (i32::from(a[0]) - i32::from(b[0])).abs()
        + (i32::from(a[1]) - i32::from(b[1])).abs()
        + (i32::from(a[2]) - i32::from(b[2])).abs()
}

/// Classifies a background pixel as sky, ground, or neither (symbology,
/// horizon anti-aliasing). The sky/ground fills sit 320 apart in Manhattan
/// space, so a tight threshold cleanly rejects every other color.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum Fill {
    Sky,
    Ground,
    Other,
}

fn classify(px: [u8; 3]) -> Fill {
    let (ds, dg) = (manhattan(px, SKY), manhattan(px, GROUND));
    if ds < 90 && ds <= dg {
        Fill::Sky
    } else if dg < 90 {
        Fill::Ground
    } else {
        Fill::Other
    }
}

/// Renders the PFD at an orientation into an RGBA framebuffer.
fn render_pfd(roll_deg: f32, pitch_deg: f32) -> Vec<u8> {
    let data = resolve(&state_at(roll_deg, pitch_deg), &FreshnessPolicy::default());
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut writer = SceneWriter::new(&mut buf).expect("writer");
    draw_pfd(&data, &PfdConfig::default(), None, &mut writer).expect("pfd");
    let n = writer.finish();
    buf.truncate(n);

    let mut fb = std::vec![0u8; (W * H * 4) as usize];
    let report = render(
        &buf,
        &mut fb,
        FramebufferDims::tight(W, H),
        FrameId::default(),
    )
    .expect("panel renders");
    assert_eq!(report.status, RenderStatus::Painted);
    fb
}

/// Roll-arc radius plus margin: the central attitude symbology (roll arc,
/// sky pointer, aircraft symbol, pitch ladder) lives inside this radius.
const SYMBOLOGY_R: f64 = 172.0;

/// A pixel sits in a clean background band — outside the central attitude
/// symbology circle and clear of the left/right speed/altitude tapes — so
/// its color is the sky or ground fill, not a symbol.
fn is_clean_bg(px: u32, py: u32) -> bool {
    let dx = f64::from(px) + 0.5 - CX;
    let dy = f64::from(py) + 0.5 - CY;
    let outside_symbology = (dx * dx + dy * dy).sqrt() > SYMBOLOGY_R;
    let clear_of_tapes = (80..400).contains(&px);
    outside_symbology && clear_of_tapes
}

/// The dominant background fill over a horizontal strip clear of the
/// central symbology and side tapes, at a given screen row.
fn strip_fill(fb: &[u8], row: u32) -> Fill {
    let (mut sky, mut ground) = (0u32, 0u32);
    let mut px = 80;
    while px < 400 {
        if is_clean_bg(px, row) {
            match classify(pixel(fb, px, row)) {
                Fill::Sky => sky += 1,
                Fill::Ground => ground += 1,
                Fill::Other => {}
            }
        }
        px += 4;
    }
    if sky > ground {
        Fill::Sky
    } else if ground > sky {
        Fill::Ground
    } else {
        Fill::Other
    }
}

#[test]
fn upright_level_paints_sky_over_ground() {
    let fb = render_pfd(0.0, 0.0);
    assert_eq!(
        strip_fill(&fb, 8),
        Fill::Sky,
        "top band is sky when upright"
    );
    assert_eq!(
        strip_fill(&fb, H - 8),
        Fill::Ground,
        "bottom band is ground when upright"
    );
}

#[test]
fn inverted_level_paints_ground_over_sky() {
    // The unambiguous inverted cue: upside down, the ground fills the top
    // of the display and the sky the bottom — the sky/ground band inverts,
    // it is not relabeled.
    let fb = render_pfd(180.0, 0.0);
    assert_eq!(
        strip_fill(&fb, 8),
        Fill::Ground,
        "top band is ground when inverted"
    );
    assert_eq!(
        strip_fill(&fb, H - 8),
        Fill::Sky,
        "bottom band is sky when inverted"
    );
}

#[test]
fn sky_ground_band_matches_the_independent_reference() {
    // Orientations away from the vertical bank-hold window, where the f64
    // atan2/asin oracle matches the rendered geometry exactly. Every clean
    // background sample must agree with the oracle's sky/ground call.
    for (roll, pitch) in [
        (0.0f32, 0.0f32),
        (30.0, 10.0),
        (-45.0, -15.0),
        (60.0, 20.0),
        (180.0, 0.0),
        (150.0, 12.0),
        (-160.0, -18.0),
        (120.0, -25.0),
    ] {
        let q = euler_quat(roll, pitch, 0.0);
        let (ref_pitch, ref_bank) = ref_pitch_bank(q);
        let fb = render_pfd(roll, pitch);
        let horizon_y = ref_pitch.to_degrees() * PX_PER_DEG_PITCH;

        let (mut checked, mut mismatches) = (0u32, 0u32);
        let mut py = 4;
        while py < H - 4 {
            let mut px = 4;
            while px < W - 4 {
                let dx = f64::from(px) + 0.5 - CX;
                let dy = f64::from(py) + 0.5 - CY;
                let y_rot = dx * ref_bank.sin() + dy * ref_bank.cos();
                let near_horizon = (y_rot - horizon_y).abs() < 24.0;
                if !near_horizon && is_clean_bg(px, py) {
                    match classify(pixel(&fb, px, py)) {
                        Fill::Sky | Fill::Ground => {
                            checked += 1;
                            let want_sky = ref_is_sky(px, py, ref_pitch, ref_bank);
                            let got_sky = classify(pixel(&fb, px, py)) == Fill::Sky;
                            if want_sky != got_sky {
                                mismatches += 1;
                            }
                        }
                        Fill::Other => {}
                    }
                }
                px += 8;
            }
            py += 8;
        }
        assert!(checked > 200, "roll {roll} pitch {pitch}: too few samples");
        assert_eq!(
            mismatches, 0,
            "roll {roll} pitch {pitch}: sky/ground pixels disagree with the f64 reference"
        );
    }
}

#[test]
fn passage_through_the_vertical_never_flips_sky_ground() {
    // Acceptance: continuous passage through ±90° must not flip sky/ground
    // meaning. Nose high through 89/90/91° keeps the display filled with
    // sky (looking up); nose low keeps it ground. A flip would swap the
    // dominant band between adjacent steps.
    for pitch in [89.0f32, 90.0, 91.0] {
        let fb = render_pfd(0.0, pitch);
        assert_eq!(
            strip_fill(&fb, 8),
            Fill::Sky,
            "nose-high {pitch}° top stays sky"
        );
        assert_eq!(
            strip_fill(&fb, H - 8),
            Fill::Sky,
            "nose-high {pitch}° bottom stays sky (no flip)"
        );
    }
    for pitch in [-89.0f32, -90.0, -91.0] {
        let fb = render_pfd(0.0, pitch);
        assert_eq!(
            strip_fill(&fb, 8),
            Fill::Ground,
            "nose-low {pitch}° top stays ground"
        );
        assert_eq!(
            strip_fill(&fb, H - 8),
            Fill::Ground,
            "nose-low {pitch}° bottom stays ground (no flip)"
        );
    }
}

#[test]
fn extreme_attitudes_render_bit_reproducibly() {
    for (roll, pitch) in [
        (0.0f32, 90.0f32),
        (180.0, 0.0),
        (179.0, -20.0),
        (90.0, 45.0),
    ] {
        let first = render_pfd(roll, pitch);
        let second = render_pfd(roll, pitch);
        assert_eq!(
            first, second,
            "roll {roll} pitch {pitch} renders identically twice"
        );
    }
}

#[test]
fn inverted_nose_high_paints_recovery_chevrons() {
    // Recovery chevrons are the only red symbology on the PFD; their
    // presence in an inverted, steeply nose-high frame confirms the cue
    // survives to the raster, pointed by the roll-rotated attitude frame.
    let fb = render_pfd(180.0, 60.0);
    let mut red = 0u32;
    let mut i = 0;
    while i + 3 < fb.len() {
        if manhattan([fb[i], fb[i + 1], fb[i + 2]], RED) < 40 && fb[i + 3] > 0 {
            red += 1;
        }
        i += 4;
    }
    assert!(red > 20, "inverted nose-high paints red recovery chevrons");
}
