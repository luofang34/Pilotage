//! HSI corner data boxes: wind, distance, course, heading select, and
//! the vertical-deviation scale.

use core::f32::consts::PI;
use pilotage_instrument_scene::{Anchor, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::units::{MPS_TO_KT, RAD_TO_DEG, wrap_deg_360};
use pilotage_instrument_state::{NavSource, PanelData};

use crate::fixed_str::fmt_label;
use crate::palette;

use super::cdi::source_color;

/// Wind vector and speed, top-left.
pub fn wind_box(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    scene.stroke(palette::GREY, 1.5)?;
    scene.rect(PaintMode::FillStroke, 2.0, 2.0, 112.0, 48.0)?;
    let wind = data.wind;
    if !wind.status.shows_value() {
        scene.fill_color(palette::GREY)?;
        scene.text(58.0, 26.0, 14.0, Anchor::CENTER, "WIND ---")?;
        return Ok(());
    }
    let hdg = data.heading.value_rad.value;
    // Arrow points where the wind blows toward, in the aircraft's frame.
    scene.save()?;
    scene.translate(28.0, 26.0)?;
    scene.rotate(wind.value.from_rad - hdg + PI)?;
    scene.stroke(palette::WHITE, 2.0)?;
    scene.line(0.0, -12.0, 0.0, 8.0)?;
    scene.fill_color(palette::WHITE)?;
    scene.polygon(PaintMode::Fill, &[[0.0, 16.0], [-5.0, 6.0], [5.0, 6.0]])?;
    scene.restore()?;

    let deg = libm::roundf(wrap_deg_360(wind.value.from_rad * RAD_TO_DEG)) as i32;
    let dir = fmt_label!(8, "{deg:03}°");
    let spd = fmt_label!(12, "{:.0}kt", wind.value.speed_mps * MPS_TO_KT);
    scene.fill_color(palette::WHITE)?;
    scene.text(78.0, 16.0, 15.0, Anchor::CENTER, dir.as_str())?;
    scene.text(78.0, 36.0, 15.0, Anchor::CENTER, spd.as_str())?;
    Ok(())
}

/// Distance to waypoint/station, top-right.
pub fn dist_box(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    scene.stroke(palette::GREY, 1.5)?;
    scene.rect(PaintMode::FillStroke, 366.0, 2.0, 112.0, 48.0)?;
    scene.fill_color(palette::GREY)?;
    scene.text(422.0, 14.0, 12.0, Anchor::CENTER, "DIST NM")?;
    match (data.nav.data.dist_nm, data.nav.status.shows_value()) {
        (Some(nm), true) => {
            let text = fmt_label!(12, "{nm:.1}");
            scene.fill_color(palette::MAGENTA)?;
            scene.text(422.0, 34.0, 18.0, Anchor::CENTER, text.as_str())?;
        }
        _ => {
            scene.fill_color(palette::GREY)?;
            scene.text(422.0, 34.0, 18.0, Anchor::CENTER, "--.-")?;
        }
    }
    Ok(())
}

/// Selected course, bottom-left, in the nav-source color.
pub fn course_box(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    scene.stroke(palette::GREY, 1.5)?;
    scene.rect(PaintMode::FillStroke, 2.0, 322.0, 112.0, 36.0)?;
    scene.fill_color(palette::WHITE)?;
    scene.text(30.0, 340.0, 15.0, Anchor::CENTER, "CRS")?;
    if data.nav.data.source == NavSource::None || !data.nav.status.shows_value() {
        scene.fill_color(palette::GREY)?;
        scene.text(78.0, 340.0, 18.0, Anchor::CENTER, "---°")?;
        return Ok(());
    }
    let deg = libm::roundf(wrap_deg_360(data.nav.data.course_rad * RAD_TO_DEG)) as i32;
    let shown = if deg == 0 { 360 } else { deg };
    let text = fmt_label!(8, "{shown:03}°");
    scene.fill_color(source_color(data.nav.data.source))?;
    scene.text(78.0, 340.0, 18.0, Anchor::CENTER, text.as_str())?;
    Ok(())
}

/// Selected heading, bottom-right, cyan like its bug.
pub fn heading_sel_box(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    scene.stroke(palette::GREY, 1.5)?;
    scene.rect(PaintMode::FillStroke, 366.0, 322.0, 112.0, 36.0)?;
    let deg = libm::roundf(wrap_deg_360(data.selections.heading_bug_rad * RAD_TO_DEG)) as i32;
    let shown = if deg == 0 { 360 } else { deg };
    let text = fmt_label!(8, "{shown:03}°");
    scene.fill_color(palette::CYAN)?;
    scene.polygon(
        PaintMode::Fill,
        &[
            [382.0, 334.0],
            [398.0, 334.0],
            [398.0, 346.0],
            [393.0, 346.0],
            [390.0, 341.0],
            [387.0, 346.0],
            [382.0, 346.0],
        ],
    )?;
    scene.text(438.0, 340.0, 18.0, Anchor::CENTER, text.as_str())?;
    Ok(())
}

/// Vertical deviation (glideslope/glidepath) scale at the right edge.
pub fn vertical_deviation(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let Some(vdev) = data.nav.data.vdev_dots else {
        return Ok(());
    };
    if !data.nav.status.shows_value() || data.nav.data.source == NavSource::None {
        return Ok(());
    }
    // ±2.5 dots over ±96 px (38.4 px/dot).
    let x = 452.0;
    scene.stroke(palette::GREY, 2.0)?;
    scene.line(x, super::CY - 96.0, x, super::CY + 96.0)?;
    for dy in [-76.8f32, -38.4, 38.4, 76.8] {
        scene.circle(PaintMode::Stroke, x, super::CY + dy, 4.0)?;
    }
    let color = source_color(data.nav.data.source);
    let y = super::CY + vdev.clamp(-2.5, 2.5) * 38.4;
    scene.fill_color(color)?;
    scene.polygon(
        PaintMode::Fill,
        &[[x, y - 9.0], [x + 7.0, y], [x, y + 9.0], [x - 7.0, y]],
    )?;
    let tag = if data.nav.data.source == NavSource::Gps {
        "V"
    } else {
        "G"
    };
    scene.fill_color(color)?;
    scene.text(x, super::CY - 108.0, 13.0, Anchor::CENTER, tag)?;
    Ok(())
}
