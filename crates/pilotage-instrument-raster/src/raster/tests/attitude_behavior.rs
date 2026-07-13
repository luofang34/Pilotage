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
/// The simulator profile's minimum reverse-color band, in degrees of pitch.
const SIM_BAND_DEG: f64 = 2.5;
/// Pitch at which the level horizon reaches the panel edge (180 px / 7.2).
const VIEWPORT_HALF_PITCH_DEG: f64 = 25.0;
/// Display pitch beyond which the background fill boundary is clamped so the
/// reverse-color band survives. Mirrors `horizon.rs`.
const FILL_LIMIT_DEG: f64 = VIEWPORT_HALF_PITCH_DEG - SIM_BAND_DEG;

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
fn state_from_quat(quat: Quat) -> AircraftState {
    let mut state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat,
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
    // The fill boundary is clamped to preserve the reverse-color band, so the
    // oracle clamps the same way; the sign (physical ground/sky side) is kept.
    let fill_deg = pitch_rad
        .to_degrees()
        .clamp(-FILL_LIMIT_DEG, FILL_LIMIT_DEG);
    let horizon_y = fill_deg * PX_PER_DEG_PITCH;
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
    render_quat(euler_quat(roll_deg, pitch_deg, 0.0))
}

/// Renders the PFD from a raw attitude quaternion into an RGBA framebuffer.
fn render_quat(quat: Quat) -> Vec<u8> {
    let data = resolve(&state_from_quat(quat), &FreshnessPolicy::default());
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

/// Sky and ground pixel counts over the clean background band.
fn bg_counts(fb: &[u8]) -> (u32, u32) {
    let (mut sky, mut ground) = (0u32, 0u32);
    let mut py = 4;
    while py < H - 4 {
        let mut px = 4;
        while px < W - 4 {
            if is_clean_bg(px, py) {
                match classify(pixel(fb, px, py)) {
                    Fill::Sky => sky += 1,
                    Fill::Ground => ground += 1,
                    Fill::Other => {}
                }
            }
            px += 6;
        }
        py += 6;
    }
    (sky, ground)
}

#[test]
fn extreme_pitch_keeps_a_reverse_color_band() {
    // Reopened ATT-01 defect: at ±89/90/91° the true horizon leaves the 360 px
    // viewport and the background used to collapse to a single flat color,
    // losing one of its two orientation cues. The clamped fill now keeps a
    // minimum band of the REVERSE color. Looking up (nose-high) sky dominates
    // with a ground band still present; looking down, ground dominates with a
    // sky band. The sky/ground MEANING never flips (the dominant field stays
    // correct through the vertical); only the band's edge follows the physical
    // orientation over the top. Both colors are always present — never one flat
    // field.
    const MIN_BAND: u32 = 40;
    for pitch in [89.0f32, 90.0, 91.0] {
        let (sky, ground) = bg_counts(&render_pfd(0.0, pitch));
        assert!(
            sky > ground,
            "nose-high {pitch}°: sky dominates (no meaning flip)"
        );
        assert!(
            ground > MIN_BAND,
            "nose-high {pitch}°: reverse-color ground band present"
        );
    }
    for pitch in [-89.0f32, -90.0, -91.0] {
        let (sky, ground) = bg_counts(&render_pfd(0.0, pitch));
        assert!(
            ground > sky,
            "nose-low {pitch}°: ground dominates (no meaning flip)"
        );
        assert!(
            sky > MIN_BAND,
            "nose-low {pitch}°: reverse-color sky band present"
        );
    }
    // At the upright extremes (just short of vertical, before the display goes
    // over the top) the band sits on the physically correct edge.
    let up = render_pfd(0.0, 89.0);
    assert_eq!(strip_fill(&up, 8), Fill::Sky, "nose-high 89°: sky above");
    assert_eq!(
        strip_fill(&up, H - 6),
        Fill::Ground,
        "nose-high 89°: ground reverse-band at the bottom edge"
    );
    let down = render_pfd(0.0, -89.0);
    assert_eq!(
        strip_fill(&down, 6),
        Fill::Sky,
        "nose-low 89°: sky reverse-band at the top edge"
    );
    assert_eq!(
        strip_fill(&down, H - 8),
        Fill::Ground,
        "nose-low 89°: ground below"
    );
}

#[test]
fn q_and_negated_q_render_identically() {
    // q and -q are one physical orientation; the SO(3)-safe presentation reads
    // only quadratic quaternion forms, so the lit frame must be byte-identical
    // — proven at the raster, across the extreme envelope and the band.
    for (roll, pitch, yaw) in [
        (0.0f32, 89.0f32, 0.0f32),
        (180.0, 0.0, 30.0),
        (150.0, 12.0, -20.0),
        (0.0, -90.0, 0.0),
    ] {
        let q = euler_quat(roll, pitch, yaw);
        let nq = Quat {
            w: -q.w,
            x: -q.x,
            y: -q.y,
            z: -q.z,
        };
        assert_eq!(
            render_quat(q),
            render_quat(nq),
            "roll {roll} pitch {pitch}: q and -q must render identically"
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
