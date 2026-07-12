//! Optional sky/ground fill and critical attitude symbology.
//!
//! Geometry follows the G5 proportions: 7.2 px per degree of pitch
//! (±25° across the 360-px panel height), roll arc radius 144 px
//! spanning ±60° of bank.

use core::f32::consts::PI;
use libm::{cosf, sinf};
use pilotage_instrument_scene::{Anchor, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::units::RAD_TO_DEG;

use crate::fixed_str::fmt_label;
use crate::palette;

const PX_PER_DEG_PITCH: f32 = 7.2;
const ROLL_ARC_R: f32 = 144.0;
/// Pitch-ladder marks are culled beyond this radius so they stay inside
/// the roll arc (the pyG5 clip).
const LADDER_MAX_Y: f32 = 104.0;

/// Optional sky/ground fill in the roll-rotated attitude frame.
pub fn draw_background(
    scene: &mut SceneWriter<'_>,
    roll_rad: f32,
    pitch_rad: f32,
) -> Result<(), SceneError> {
    let pitch_deg = pitch_rad * RAD_TO_DEG;
    let horizon_y = pitch_deg * PX_PER_DEG_PITCH;

    scene.save()?;
    scene.translate(240.0, 180.0)?;
    scene.rotate(-roll_rad)?;

    // Oversized halves so any roll/pitch combination stays covered.
    scene.fill_color(palette::SKY)?;
    scene.rect(PaintMode::Fill, -600.0, -600.0, 1200.0, 600.0 + horizon_y)?;
    scene.fill_color(palette::GROUND)?;
    scene.rect(PaintMode::Fill, -600.0, horizon_y, 1200.0, 1200.0)?;
    scene.restore()?;
    Ok(())
}

/// Critical horizon line and pitch ladder in the attitude frame.
pub fn draw_horizon_cues(
    scene: &mut SceneWriter<'_>,
    roll_rad: f32,
    pitch_rad: f32,
    declutter: bool,
) -> Result<(), SceneError> {
    let horizon_y = pitch_rad * RAD_TO_DEG * PX_PER_DEG_PITCH;
    scene.save()?;
    scene.translate(240.0, 180.0)?;
    scene.rotate(-roll_rad)?;
    scene.stroke(palette::WHITE, 2.0)?;
    scene.line(-600.0, horizon_y, 600.0, horizon_y)?;
    draw_pitch_ladder(scene, horizon_y, declutter)?;
    scene.restore()?;
    Ok(())
}

/// Ladder marks every 2.5°, wide labeled bars every 10° (pyG5's
/// half-width cycle 10/20/10/30).
fn draw_pitch_ladder(
    scene: &mut SceneWriter<'_>,
    horizon_y: f32,
    declutter: bool,
) -> Result<(), SceneError> {
    scene.stroke(palette::WHITE, 2.0)?;
    scene.fill_color(palette::WHITE)?;
    for i in -36..=36i32 {
        if i == 0 {
            continue;
        }
        // Declutter keeps only the labeled 10° bars: fewer, larger cues
        // when orientation is the problem being solved.
        if declutter && i.rem_euclid(4) != 0 {
            continue;
        }
        let deg = i as f32 * 2.5;
        let y = horizon_y - deg * PX_PER_DEG_PITCH;
        if y.abs() > LADDER_MAX_Y {
            continue;
        }
        let half = match i.rem_euclid(4) {
            0 => 30.0,
            2 => 20.0,
            _ => 10.0,
        };
        scene.line(-half, y, half, y)?;
        if i.rem_euclid(4) == 0 {
            let label = fmt_label!(8, "{}", deg.abs() as i32);
            scene.text(-half - 16.0, y, 14.0, Anchor::CENTER, label.as_str())?;
            scene.text(half + 16.0, y, 14.0, Anchor::CENTER, label.as_str())?;
        }
    }
    Ok(())
}

/// Fixed roll arc with bank ticks, plus the sky pointer that rotates with
/// the horizon.
pub fn draw_roll_scale(scene: &mut SceneWriter<'_>, roll_rad: f32) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(240.0, 180.0)?;

    scene.stroke(palette::WHITE, 2.0)?;
    // Top arc: ±60° of bank around straight up (-90° in y-down screen
    // angles).
    scene.arc(
        0.0,
        0.0,
        ROLL_ARC_R,
        -150.0 * PI / 180.0,
        120.0 * PI / 180.0,
    )?;
    const TICKS: [(f32, f32); 10] = [
        (-60.0, 12.0),
        (-45.0, 12.0),
        (-30.0, 12.0),
        (-20.0, 6.0),
        (-10.0, 6.0),
        (10.0, 6.0),
        (20.0, 6.0),
        (30.0, 12.0),
        (45.0, 12.0),
        (60.0, 12.0),
    ];
    for (bank, len) in TICKS {
        let a = (bank - 90.0) * PI / 180.0;
        let (c, s) = (cosf(a), sinf(a));
        scene.line(
            ROLL_ARC_R * c,
            ROLL_ARC_R * s,
            (ROLL_ARC_R + len) * c,
            (ROLL_ARC_R + len) * s,
        )?;
    }
    // Fixed zero-bank triangle at the arc apex, pointing down at the sky
    // pointer.
    scene.fill_color(palette::WHITE)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [0.0, -ROLL_ARC_R],
            [-7.0, -ROLL_ARC_R - 14.0],
            [7.0, -ROLL_ARC_R - 14.0],
        ],
    )?;

    // Sky pointer: rides the horizon.
    scene.rotate(-roll_rad)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [0.0, -ROLL_ARC_R + 2.0],
            [-9.0, -ROLL_ARC_R + 18.0],
            [9.0, -ROLL_ARC_R + 18.0],
        ],
    )?;
    scene.restore()?;
    Ok(())
}

/// The fixed yellow aircraft reference: wing bars and center chevron.
pub fn draw_aircraft_symbol(scene: &mut SceneWriter<'_>) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(240.0, 180.0)?;
    scene.fill_color(palette::YELLOW)?;
    scene.stroke(palette::BLACK, 1.0)?;
    scene.polygon(
        PaintMode::FillStroke,
        &[[-150.0, -4.0], [-92.0, -4.0], [-84.0, 4.0], [-150.0, 4.0]],
    )?;
    scene.polygon(
        PaintMode::FillStroke,
        &[[150.0, -4.0], [92.0, -4.0], [84.0, 4.0], [150.0, 4.0]],
    )?;
    scene.polygon(
        PaintMode::FillStroke,
        &[[0.0, 0.0], [-45.0, 22.0], [-36.0, 22.0], [0.0, 8.0]],
    )?;
    scene.polygon(
        PaintMode::FillStroke,
        &[[0.0, 0.0], [45.0, 22.0], [36.0, 22.0], [0.0, 8.0]],
    )?;
    scene.restore()?;
    Ok(())
}
