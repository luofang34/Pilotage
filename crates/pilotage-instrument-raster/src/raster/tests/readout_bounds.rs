#![allow(clippy::expect_used, clippy::panic)]
//! DISP-02 guardrail: across the representable value range, the tape
//! readouts' glyph runs stay inside their pointed boxes.
//!
//! The check is semantic, not pixel inspection: the PFD scene is
//! decoded, each pointed readout box is located from its own polygon,
//! and the following text run's extents are recomputed from the glyph
//! manifest — the reference backend's actual metrics. A run whose ink
//! would leave the box body fails here for the value that overflows,
//! not as an unexplained pixel diff. A drift guard binds the scene
//! text-metrics contract to those manifest metrics, so the panel-side
//! fitting and the backend-side painting cannot diverge silently.

use pilotage_instrument_glyphs::{ADVANCE, CELL_H, CELL_W};
use pilotage_instrument_panels::{PANEL_W, PfdConfig, draw_pfd};
use pilotage_instrument_scene::{
    Anchor, Cmd, MAX_SCENE_BYTES, SceneCmds, SceneWriter, nominal_text_ink_width,
    nominal_text_width,
};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, EstimateQuality, FreshnessPolicy, Kinematics, Quat, Stamped,
    ValidFlags, resolve,
};
use std::string::{String, ToString};
use std::vec::Vec;

const FT_PER_M: f32 = 3.280_84;
const MPS_PER_KT: f32 = 0.514_444;

/// A level, valid flying state at the given altitude and airspeed.
fn state_at(alt_ft: f32, ias_kt: f32) -> AircraftState {
    let mut state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
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
            pos_ned_m: [0.0, 0.0, -alt_ft / FT_PER_M],
            vel_ned_mps: [40.0, 0.0, 0.0],
        }),
        age_ms: Some(20.0),
    };
    state.air = Stamped {
        data: Some(AirData {
            ias_mps: Some(ias_kt * MPS_PER_KT),
            baro_setting_hpa: Some(1013.0),
        }),
        age_ms: Some(20.0),
    };
    state
}

fn pfd_scene(alt_ft: f32, ias_kt: f32) -> Vec<u8> {
    let data = resolve(&state_at(alt_ft, ias_kt), &FreshnessPolicy::default());
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut writer = SceneWriter::new(&mut buf).expect("writer");
    draw_pfd(&data, &PfdConfig::default(), None, &mut writer).expect("pfd");
    let n = writer.finish();
    buf.truncate(n);
    buf
}

/// One located readout: its box body span and the run that followed it.
struct ReadoutRun {
    body_left: f32,
    body_right: f32,
    x: f32,
    size: f32,
    text: String,
}

/// The pointed-readout y profile: body corners at 155/205, shoulders at
/// 168/192, tip vertex at 180.
fn is_pointed_box(points: &[[f32; 2]]) -> bool {
    points.len() == 7
        && points
            .iter()
            .all(|p| [155.0, 168.0, 180.0, 192.0, 205.0].contains(&p[1]))
        && points.iter().filter(|p| p[1] == 180.0).count() == 1
}

/// Decodes the scene and pairs every pointed readout box with the text
/// run drawn after it (the readout value is the next text command the
/// panel emits).
fn readout_runs(bytes: &[u8]) -> Vec<ReadoutRun> {
    let mut runs = Vec::new();
    let mut pending: Option<(f32, f32)> = None;
    for cmd in SceneCmds::new(bytes).expect("decode") {
        match cmd.expect("cmd") {
            Cmd::Polygon { points, .. } => {
                let pts: Vec<[f32; 2]> = points.iter().collect();
                if is_pointed_box(&pts) {
                    let tip_y = 180.0;
                    let body: Vec<f32> =
                        pts.iter().filter(|p| p[1] != tip_y).map(|p| p[0]).collect();
                    let left = body.iter().copied().fold(f32::INFINITY, f32::min);
                    let right = body.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    pending = Some((left, right));
                }
            }
            Cmd::Text {
                x,
                size,
                anchor,
                text,
                ..
            } => {
                if let Some((body_left, body_right)) = pending.take() {
                    assert_eq!(anchor, Anchor::CENTER, "readout anchor");
                    runs.push(ReadoutRun {
                        body_left,
                        body_right,
                        x,
                        size,
                        text: text.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    runs
}

/// Ink extents of a run exactly as the reference backend paints it
/// (`raster::text::draw_run`): pen advances `ADVANCE`-scaled per char,
/// glyph ink spans `CELL_W`-scaled from the pen.
fn ink_extents(run: &ReadoutRun) -> (f32, f32) {
    let scale = run.size / CELL_H as f32;
    let advance = f32::from(ADVANCE) * scale;
    let chars = run.text.chars().count() as f32;
    let width = chars * advance;
    let left = run.x - width / 2.0;
    let ink_right = left + width - (f32::from(ADVANCE) - CELL_W as f32) * scale;
    (left, ink_right)
}

/// Altitude sweep: the full sign + five digit envelope after the 10-ft
/// rounding — including the widest negative ("-99990"), both sides of
/// every digit-count transition (±9,990 → ±10,000), and the live-defect
/// value 1,030 ft that painted outside its box.
const ALT_SWEEP_FT: [f32; 15] = [
    -99_990.0, -10_000.0, -9_990.0, -1_030.0, -150.0, -10.0, 0.0, 990.0, 1_030.0, 2_450.0, 9_990.0,
    10_000.0, 12_340.0, 45_670.0, 99_990.0,
];
/// Airspeed sweep: the `{:03}` format floor up through four digits.
const IAS_SWEEP_KT: [f32; 6] = [0.0, 78.0, 145.0, 460.0, 999.0, 1_043.0];

#[test]
fn readout_runs_stay_inside_their_boxes_across_the_value_range() {
    const TOLERANCE: f32 = 1e-3;
    for alt_ft in ALT_SWEEP_FT {
        for ias_kt in IAS_SWEEP_KT {
            let scene = pfd_scene(alt_ft, ias_kt);
            let runs = readout_runs(&scene);
            assert_eq!(runs.len(), 2, "both tape readouts at {alt_ft}/{ias_kt}");
            for run in &runs {
                assert!(!run.text.is_empty(), "readout shows a value");
                let (left, ink_right) = ink_extents(run);
                assert!(
                    left >= run.body_left - TOLERANCE && ink_right <= run.body_right + TOLERANCE,
                    "run '{}' ink [{left}, {ink_right}] leaves box body \
                     [{}, {}] at alt {alt_ft} ft / ias {ias_kt} kt",
                    run.text,
                    run.body_left,
                    run.body_right,
                );
                assert!(
                    left >= 0.0 && ink_right <= PANEL_W,
                    "run '{}' leaves the panel at alt {alt_ft} ft / ias {ias_kt} kt",
                    run.text,
                );
            }
        }
    }
}

#[test]
fn the_defect_value_renders_all_its_digits() {
    // 1,030 ft rounds to "1030": the readout must carry every digit —
    // shrinking is legal, truncating or clipping is not.
    let scene = pfd_scene(1_030.0, 78.0);
    let runs = readout_runs(&scene);
    let alt = runs
        .iter()
        .find(|r| r.body_left > 300.0)
        .expect("altitude readout");
    assert_eq!(alt.text, "1030");
}

#[test]
fn scene_text_metrics_contract_matches_the_glyph_manifest() {
    for (size, chars) in [(26.0f32, 1usize), (28.0, 3), (21.0, 4), (14.0, 6)] {
        let scale = size / CELL_H as f32;
        let manifest_width = chars as f32 * f32::from(ADVANCE) * scale;
        let manifest_ink = manifest_width - (f32::from(ADVANCE) - CELL_W as f32) * scale;
        assert!(
            (nominal_text_width(size, chars) - manifest_width).abs() < 1e-4,
            "advance contract drifted from the glyph manifest",
        );
        assert!(
            (nominal_text_ink_width(size, chars) - manifest_ink).abs() < 1e-4,
            "ink contract drifted from the glyph manifest",
        );
    }
}

/// The worst-width corners of the sweep, re-checked at the PIXEL level:
/// each readout run is re-drawn in isolation and rasterized by the real
/// reference backend, and every painted glyph pixel must land inside the
/// box body (one pixel of quantization slack) and the panel. The
/// semantic extent test above cannot catch a regression in raster
/// anchoring, quantization, or glyph painting; this one does.
const PIXEL_CASES: [(f32, f32); 3] = [(-99_990.0, 1_043.0), (1_030.0, 78.0), (99_990.0, 999.0)];

#[test]
fn readout_glyphs_paint_only_inside_their_boxes() {
    let (w, h) = (PANEL_W as u32, 360u32);
    for (alt_ft, ias_kt) in PIXEL_CASES {
        let runs = readout_runs(&pfd_scene(alt_ft, ias_kt));
        assert_eq!(runs.len(), 2, "both readouts at {alt_ft}/{ias_kt}");
        for run in &runs {
            // The run in isolation: any lit pixel is glyph ink.
            let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
            let mut writer = SceneWriter::new(&mut buf).expect("writer");
            writer
                .begin_layer(pilotage_instrument_scene::LayerId::Tapes)
                .expect("begin layer");
            writer
                .fill_color(pilotage_instrument_scene::Rgba8 {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                })
                .expect("fill");
            writer
                .text(run.x, 180.0, run.size, Anchor::CENTER, &run.text)
                .expect("text");
            writer
                .end_layer(pilotage_instrument_scene::LayerId::Tapes)
                .expect("end layer");
            let n = writer.finish();
            buf.truncate(n);

            let mut fb = std::vec![0u8; (w * h * 4) as usize];
            let report = crate::render(
                &buf,
                &mut fb,
                crate::FramebufferDims::tight(w, h),
                crate::FrameId::default(),
            )
            .expect("isolated run renders");
            assert_eq!(report.status, crate::RenderStatus::Painted);

            let mut lit = 0usize;
            for py in 0..h {
                for px in 0..w {
                    let i = ((py * w + px) * 4) as usize;
                    if fb[i] == 0 && fb[i + 1] == 0 && fb[i + 2] == 0 {
                        continue;
                    }
                    lit += 1;
                    let x = px as f32;
                    assert!(
                        x >= run.body_left - 1.0 && x + 1.0 <= run.body_right + 1.0,
                        "glyph pixel ({px},{py}) of '{}' outside box body [{}, {}] \
                         at alt {alt_ft} ft / ias {ias_kt} kt",
                        run.text,
                        run.body_left,
                        run.body_right,
                    );
                    assert!(x + 1.0 <= PANEL_W, "glyph pixel outside the panel");
                }
            }
            assert!(lit > 0, "run '{}' painted no pixels", run.text);
        }
    }
}
