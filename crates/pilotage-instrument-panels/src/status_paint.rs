//! Shared honest-status rendering: flags, dashes, and the red X.

use pilotage_instrument_scene::{Anchor, PaintMode, Rgba8, SceneError, SceneWriter};
use pilotage_instrument_state::SignalStatus;

use crate::palette;

/// The accent color a status imposes on its readout, `None` for normal.
pub fn status_accent(status: SignalStatus) -> Option<Rgba8> {
    match status {
        SignalStatus::Valid => None,
        SignalStatus::Degraded | SignalStatus::Stale => Some(palette::AMBER),
        SignalStatus::Missing | SignalStatus::Failed => Some(palette::RED),
    }
}

/// Draws the red-X failure flag over a rectangular instrument region,
/// with a label naming what failed.
pub fn draw_red_x(
    scene: &mut SceneWriter<'_>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
) -> Result<(), SceneError> {
    scene.stroke(palette::RED, 4.0)?;
    scene.line(x, y, x + w, y + h)?;
    scene.line(x + w, y, x, y + h)?;
    scene.fill_color(palette::RED)?;
    scene.text(x + w / 2.0, y + h / 2.0, 20.0, Anchor::CENTER, label)?;
    Ok(())
}

/// Draws an amber stale/degraded flag tag near a readout.
pub fn draw_flag(
    scene: &mut SceneWriter<'_>,
    x: f32,
    y: f32,
    label: &str,
) -> Result<(), SceneError> {
    scene.fill_color(palette::AMBER)?;
    scene.text(x, y, 11.0, Anchor::CENTER, label)?;
    Ok(())
}

/// Draws a readout box: filled background, status-colored border, and
/// either the value text or dashes when the status hides the value.
#[allow(clippy::too_many_arguments)]
pub fn readout_box(
    scene: &mut SceneWriter<'_>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &str,
    text_color: Rgba8,
    size: f32,
    status: SignalStatus,
) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    let border = status_accent(status).unwrap_or(palette::GREY);
    scene.stroke(border, 1.5)?;
    scene.rect(PaintMode::FillStroke, x, y, w, h)?;
    if status.shows_value() {
        scene.fill_color(text_color)?;
        scene.text(x + w / 2.0, y + h / 2.0, size, Anchor::CENTER, text)?;
    } else {
        scene.fill_color(palette::RED)?;
        scene.text(x + w / 2.0, y + h / 2.0, size, Anchor::CENTER, "---")?;
    }
    Ok(())
}
