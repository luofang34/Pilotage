//! The Horizontal Situation Indicator: rotating compass rose, heading
//! bug, ground-track diamond, course deviation indicator, and data boxes.

use pilotage_instrument_scene::{PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{NavSource, PanelData};

use crate::palette;
use crate::status_paint;
use crate::{PANEL_H, PANEL_W};

mod boxes;
mod cdi;
mod rose;

/// Rose center; below panel center to leave room for the heading box.
pub(crate) const CX: f32 = 240.0;
/// Rose center Y.
pub(crate) const CY: f32 = 190.0;
/// Compass rose radius.
pub(crate) const ROSE_R: f32 = 160.0;

/// Draws the HSI from resolved state.
pub fn draw_hsi(data: &PanelData, scene: &mut SceneWriter<'_>) -> Result<(), SceneError> {
    scene.fill_color(palette::BLACK)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;

    let hdg = data.heading_rad;
    if hdg.status.shows_value() {
        rose::draw_rose(scene, hdg.value)?;
        rose::draw_heading_bug(scene, hdg.value, data.selections.heading_bug_rad)?;
        if data.track_rad.status.shows_value() {
            rose::draw_track_diamond(scene, hdg.value, data.track_rad.value)?;
        }
        if data.nav.data.source != NavSource::None && data.nav.status.shows_value() {
            cdi::draw_cdi(scene, &data.nav, hdg.value)?;
        }
    } else {
        status_paint::draw_red_x(scene, CX - 140.0, CY - 140.0, 280.0, 280.0, "HDG")?;
    }

    rose::draw_heading_box(scene, hdg)?;
    boxes::wind_box(scene, data)?;
    boxes::dist_box(scene, data)?;
    boxes::course_box(scene, data)?;
    boxes::heading_sel_box(scene, data)?;
    boxes::vertical_deviation(scene, data)?;
    Ok(())
}

#[cfg(test)]
mod tests;
