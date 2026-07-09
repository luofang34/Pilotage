//! The rotating compass rose, heading readout, bug, and track diamond.

use core::f32::consts::PI;
use libm::{cosf, sinf};
use pilotage_instrument_scene::{Anchor, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::Sig;
use pilotage_instrument_state::units::{RAD_TO_DEG, wrap_deg_360};

use crate::fixed_str::fmt_label;
use crate::palette;
use crate::status_paint;

use super::{CX, CY, ROSE_R};

const TEXT_R: f32 = 126.0;

/// Rose ticks rotate with heading; labels are drawn upright at computed
/// positions (the pyG5 counter-rotation, without nested transforms).
pub fn draw_rose(scene: &mut SceneWriter<'_>, heading_rad: f32) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(CX, CY)?;

    // Fixed 45° reference marks outside the rose.
    scene.stroke(palette::GREY, 2.0)?;
    for d in [45.0f32, 90.0, 135.0, 225.0, 270.0, 315.0] {
        let a = (d - 90.0) * PI / 180.0;
        let (c, s) = (cosf(a), sinf(a));
        scene.line(
            ROSE_R * c,
            ROSE_R * s,
            (ROSE_R + 8.0) * c,
            (ROSE_R + 8.0) * s,
        )?;
    }

    // Rotating tick ring.
    scene.save()?;
    scene.rotate(-heading_rad)?;
    scene.stroke(palette::WHITE, 2.0)?;
    for i in 0..72u32 {
        let len = if i % 2 == 0 { 14.0 } else { 8.0 };
        let a = (i as f32 * 5.0 - 90.0) * PI / 180.0;
        let (c, s) = (cosf(a), sinf(a));
        scene.line(
            ROSE_R * c,
            ROSE_R * s,
            (ROSE_R - len) * c,
            (ROSE_R - len) * s,
        )?;
    }
    scene.restore()?;

    // Upright labels every 30°.
    scene.fill_color(palette::WHITE)?;
    for i in 0..12u32 {
        let deg = i * 30;
        let a = (deg as f32 - heading_rad * RAD_TO_DEG - 90.0) * PI / 180.0;
        let (x, y) = (TEXT_R * cosf(a), TEXT_R * sinf(a));
        let label = match deg {
            0 => fmt_label!(4, "N"),
            90 => fmt_label!(4, "E"),
            180 => fmt_label!(4, "S"),
            270 => fmt_label!(4, "W"),
            other => fmt_label!(4, "{}", other / 10),
        };
        let size = if deg % 90 == 0 { 22.0 } else { 17.0 };
        scene.text(x, y, size, Anchor::CENTER, label.as_str())?;
    }

    // Fixed lubber triangle at the top of the rose.
    scene.fill_color(palette::WHITE)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [0.0, -ROSE_R + 2.0],
            [-7.0, -ROSE_R - 10.0],
            [7.0, -ROSE_R - 10.0],
        ],
    )?;
    scene.restore()?;
    Ok(())
}

/// The digital heading readout box at the panel top.
pub fn draw_heading_box(scene: &mut SceneWriter<'_>, hdg: Sig<f32>) -> Result<(), SceneError> {
    let deg = libm::roundf(wrap_deg_360(hdg.value * RAD_TO_DEG)) as i32;
    let shown = if deg == 0 { 360 } else { deg };
    let text = fmt_label!(8, "{shown:03}°");
    status_paint::readout_box(
        scene,
        CX - 34.0,
        2.0,
        68.0,
        26.0,
        text.as_str(),
        palette::WHITE,
        20.0,
        hdg.status,
    )
}

/// The cyan heading bug on the rose rim.
pub fn draw_heading_bug(
    scene: &mut SceneWriter<'_>,
    heading_rad: f32,
    bug_rad: f32,
) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(CX, CY)?;
    scene.rotate(bug_rad - heading_rad)?;
    scene.fill_color(palette::CYAN)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [-9.0, -ROSE_R],
            [9.0, -ROSE_R],
            [9.0, -ROSE_R + 8.0],
            [4.0, -ROSE_R + 8.0],
            [0.0, -ROSE_R + 3.0],
            [-4.0, -ROSE_R + 8.0],
            [-9.0, -ROSE_R + 8.0],
        ],
    )?;
    scene.restore()?;
    Ok(())
}

/// The magenta ground-track diamond on the rose rim.
pub fn draw_track_diamond(
    scene: &mut SceneWriter<'_>,
    heading_rad: f32,
    track_rad: f32,
) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(CX, CY)?;
    scene.rotate(track_rad - heading_rad)?;
    scene.fill_color(palette::MAGENTA)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [0.0, -ROSE_R + 2.0],
            [6.0, -ROSE_R + 12.0],
            [0.0, -ROSE_R + 22.0],
            [-6.0, -ROSE_R + 12.0],
        ],
    )?;
    scene.restore()?;
    Ok(())
}
