//! The Horizontal Situation Indicator: rotating compass rose, heading
//! bug, ground-track diamond, course deviation indicator, and data boxes.

use pilotage_instrument_scene::{LayerId, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{NavSource, PanelData, SignalStatus};

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

/// Draws the HSI from resolved state in the scene-layer bands: the
/// black backdrop, the rotating orientation symbology, the readout
/// boxes, course guidance, and — above everything it annunciates — the
/// heading failure flag.
pub fn draw_hsi(data: &PanelData, scene: &mut SceneWriter<'_>) -> Result<(), SceneError> {
    scene.begin_layer(LayerId::Background)?;
    scene.fill_color(palette::BLACK)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;
    scene.end_layer(LayerId::Background)?;

    let hdg = data.heading_rad;
    scene.begin_layer(LayerId::Attitude)?;
    if hdg.status.shows_value() {
        rose::draw_rose(scene, hdg.value)?;
        rose::draw_heading_bug(scene, hdg.value, data.selections.heading_bug_rad)?;
        if data.track_rad.status.shows_value() {
            rose::draw_track_diamond(scene, hdg.value, data.track_rad.value)?;
        }
    }
    scene.end_layer(LayerId::Attitude)?;

    scene.begin_layer(LayerId::Tapes)?;
    rose::draw_heading_box(scene, hdg)?;
    boxes::wind_box(scene, data)?;
    boxes::dist_box(scene, data)?;
    boxes::course_box(scene, data)?;
    boxes::heading_sel_box(scene, data)?;
    scene.end_layer(LayerId::Tapes)?;

    scene.begin_layer(LayerId::Guidance)?;
    if hdg.status.shows_value()
        && data.nav.data.source != NavSource::None
        && data.nav.status.shows_value()
    {
        cdi::draw_cdi(scene, &data.nav, hdg.value)?;
    }
    boxes::vertical_deviation(scene, data)?;
    scene.end_layer(LayerId::Guidance)?;

    scene.begin_layer(LayerId::Annunciation)?;
    if data.nav.data.source != NavSource::None
        && data.nav.status.shows_value()
        && data.nav.status != SignalStatus::Valid
    {
        status_paint::draw_flag(scene, CX, CY + 60.0, "NAV")?;
    }
    if !hdg.status.shows_value() {
        status_paint::draw_red_x(scene, CX - 140.0, CY - 140.0, 280.0, 280.0, "HDG")?;
    }
    scene.end_layer(LayerId::Annunciation)?;
    Ok(())
}

#[cfg(test)]
mod tests;
