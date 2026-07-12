//! Speed and altitude tapes and the vertical-speed indicator.
//!
//! Tape scaling from the G5 proportions: speed ±25 kt over the panel
//! height (7.2 px/kt), altitude ±150 ft (1.2 px/ft), VSI ±1500 fpm full
//! scale.

use pilotage_instrument_scene::{Anchor, PaintMode, Rgba8, SceneError, SceneWriter};
use pilotage_instrument_state::{PanelData, Sig};

use crate::fixed_str::fmt_label;
use crate::palette;
use crate::status_paint;

use super::VSpeeds;

const PX_PER_KT: f32 = 7.2;
const PX_PER_FT: f32 = 1.2;
const CENTER_Y: f32 = 180.0;

/// Left-edge airspeed tape with bands, readout, and groundspeed box.
pub fn speed_tape(
    scene: &mut SceneWriter<'_>,
    data: &PanelData,
    v: Option<&VSpeeds>,
) -> Result<(), SceneError> {
    let ias = data.ias_kt;
    scene.fill_color(palette::TAPE_BG)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, 90.0, 335.0)?;

    if ias.status.shows_value() {
        if let Some(v) = v {
            speed_bands(scene, ias.value, v)?;
        }
        scene.save()?;
        scene.clip_rect(0.0, 0.0, 90.0, 335.0)?;
        scene.stroke(palette::WHITE, 2.0)?;
        scene.fill_color(palette::WHITE)?;
        let lo = (((ias.value - 26.0) / 5.0) as i32).max(0);
        let hi = ((ias.value + 26.0) / 5.0) as i32;
        for step in lo..=hi {
            let kt = step * 5;
            let y = CENTER_Y - (kt as f32 - ias.value) * PX_PER_KT;
            scene.line(78.0, y, 90.0, y)?;
            if step % 2 == 0 {
                let label = fmt_label!(8, "{kt}");
                scene.text(70.0, y, 20.0, Anchor::CENTER, label.as_str())?;
            }
        }
        scene.restore()?;
    } else {
        scene.fill_color(palette::GREY)?;
        scene.text(45.0, 130.0, 16.0, Anchor::CENTER, "IAS")?;
    }

    // Pointed readout box, always drawn so `Missing` shows dashes.
    let text = fmt_label!(8, "{:03}", libm::roundf(ias.value) as i32);
    pointed_box_right(scene, ias, text.as_str())?;

    // Groundspeed box under the tape.
    let gs = data.gs_kt;
    let gs_text = fmt_label!(12, "GS {:.0}kt", gs.value);
    status_paint::readout_box(
        scene,
        0.0,
        335.0,
        90.0,
        25.0,
        gs_text.as_str(),
        palette::MAGENTA,
        16.0,
        gs.status,
    )?;
    Ok(())
}

fn speed_bands(scene: &mut SceneWriter<'_>, ias: f32, v: &VSpeeds) -> Result<(), SceneError> {
    let segs: [(f32, f32, Rgba8); 3] = [
        (v.vs_kt, v.vno_kt, palette::BAND_GREEN),
        (v.vno_kt, v.vne_kt, palette::BAND_YELLOW),
        (v.vne_kt, v.vne_kt + 1000.0, palette::RED),
    ];
    for (lo, hi, color) in segs {
        band_rect(scene, ias, lo, hi, 86.0, 4.0, color)?;
    }
    band_rect(scene, ias, v.vs0_kt, v.vfe_kt, 82.0, 4.0, palette::WHITE)?;
    Ok(())
}

fn band_rect(
    scene: &mut SceneWriter<'_>,
    ias: f32,
    lo_kt: f32,
    hi_kt: f32,
    x: f32,
    w: f32,
    color: Rgba8,
) -> Result<(), SceneError> {
    let y_top = (CENTER_Y - (hi_kt - ias) * PX_PER_KT).max(0.0);
    let y_bot = (CENTER_Y - (lo_kt - ias) * PX_PER_KT).min(335.0);
    if y_bot > y_top {
        scene.fill_color(color)?;
        scene.rect(PaintMode::Fill, x, y_top, w, y_bot - y_top)?;
    }
    Ok(())
}

fn pointed_box_right(
    scene: &mut SceneWriter<'_>,
    sig: Sig<f32>,
    text: &str,
) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    let border = status_paint::status_accent(sig.status).unwrap_or(palette::WHITE);
    scene.stroke(border, 2.0)?;
    scene.polygon(
        PaintMode::FillStroke,
        &[
            [2.0, 155.0],
            [75.0, 155.0],
            [75.0, 168.0],
            [90.0, 180.0],
            [75.0, 192.0],
            [75.0, 205.0],
            [2.0, 205.0],
        ],
    )?;
    scene.fill_color(if sig.status.shows_value() {
        palette::WHITE
    } else {
        palette::RED
    })?;
    let shown = if sig.status.shows_value() {
        text
    } else {
        "---"
    };
    scene.text(40.0, 180.0, 28.0, Anchor::CENTER, shown)?;
    Ok(())
}

/// Right-edge altitude tape with selected-altitude bug and baro box.
pub fn altitude_tape(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let alt = data.alt_ft;
    scene.fill_color(palette::TAPE_BG)?;
    scene.rect(PaintMode::Fill, 390.0, 0.0, 90.0, 335.0)?;

    if alt.status.shows_value() {
        scene.save()?;
        scene.clip_rect(390.0, 0.0, 90.0, 335.0)?;
        scene.stroke(palette::WHITE, 2.0)?;
        scene.fill_color(palette::WHITE)?;
        let lo = ((alt.value - 155.0) / 20.0) as i32;
        let hi = ((alt.value + 155.0) / 20.0) as i32;
        for step in lo..=hi {
            let ft = step * 20;
            let y = CENTER_Y - (ft as f32 - alt.value) * PX_PER_FT;
            scene.line(390.0, y, 400.0, y)?;
            if step.rem_euclid(5) == 0 {
                let label = fmt_label!(12, "{ft}");
                scene.text(408.0, y, 18.0, Anchor::MIDDLE_LEFT, label.as_str())?;
            }
        }
        if let Some(sel_m) = data.selections.altitude_sel_m {
            let sel_ft = sel_m * pilotage_instrument_state::units::M_TO_FT;
            let y = (CENTER_Y - (sel_ft - alt.value) * PX_PER_FT).clamp(4.0, 331.0);
            scene.fill_color(palette::CYAN)?;
            scene.polygon(
                PaintMode::Fill,
                &[
                    [390.0, y - 8.0],
                    [398.0, y - 8.0],
                    [398.0, y - 3.0],
                    [393.0, y],
                    [398.0, y + 3.0],
                    [398.0, y + 8.0],
                    [390.0, y + 8.0],
                ],
            )?;
        }
        scene.restore()?;
    } else {
        scene.fill_color(palette::GREY)?;
        scene.text(435.0, 130.0, 16.0, Anchor::CENTER, "ALT")?;
    }

    let text = fmt_label!(12, "{}", libm::roundf(alt.value / 10.0) as i64 * 10);
    pointed_box_left(scene, alt, text.as_str())?;
    baro_and_sel_boxes(scene, data)?;
    Ok(())
}

fn pointed_box_left(
    scene: &mut SceneWriter<'_>,
    sig: Sig<f32>,
    text: &str,
) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    let border = status_paint::status_accent(sig.status).unwrap_or(palette::WHITE);
    scene.stroke(border, 2.0)?;
    scene.polygon(
        PaintMode::FillStroke,
        &[
            [478.0, 155.0],
            [405.0, 155.0],
            [405.0, 168.0],
            [390.0, 180.0],
            [405.0, 192.0],
            [405.0, 205.0],
            [478.0, 205.0],
        ],
    )?;
    scene.fill_color(if sig.status.shows_value() {
        palette::WHITE
    } else {
        palette::RED
    })?;
    let shown = if sig.status.shows_value() {
        text
    } else {
        "---"
    };
    scene.text(442.0, 180.0, 26.0, Anchor::CENTER, shown)?;
    Ok(())
}

fn baro_and_sel_boxes(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let baro = data.baro_hpa;
    let baro_text = fmt_label!(12, "{:.0}", baro.value);
    status_paint::readout_box(
        scene,
        390.0,
        335.0,
        90.0,
        25.0,
        baro_text.as_str(),
        palette::CYAN,
        16.0,
        baro.status,
    )?;
    if let Some(sel_m) = data.selections.altitude_sel_m {
        let sel_ft = sel_m * pilotage_instrument_state::units::M_TO_FT;
        let text = fmt_label!(12, "{}", libm::roundf(sel_ft) as i64);
        scene.fill_color(palette::BOX_BG)?;
        scene.stroke(palette::GREY, 1.5)?;
        scene.rect(PaintMode::FillStroke, 390.0, 0.0, 90.0, 24.0)?;
        scene.fill_color(palette::CYAN)?;
        scene.text(435.0, 12.0, 18.0, Anchor::CENTER, text.as_str())?;
    }
    Ok(())
}

/// Vertical-speed bar at the right edge of the altitude tape.
pub fn vsi(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let v = data.vsi_fpm;
    scene.stroke(palette::GREY, 1.0)?;
    for dy in [-120.0f32, -60.0, 60.0, 120.0] {
        scene.line(466.0, CENTER_Y + dy, 474.0, CENTER_Y + dy)?;
    }
    if !v.status.shows_value() {
        return Ok(());
    }
    // ±1500 fpm full scale over 180 px.
    let len = (v.value / 1500.0 * 180.0).clamp(-170.0, 170.0);
    scene.fill_color(palette::MAGENTA)?;
    if len >= 0.0 {
        scene.rect(PaintMode::Fill, 466.0, CENTER_Y - len, 8.0, len.max(1.0))?;
    } else {
        scene.rect(PaintMode::Fill, 466.0, CENTER_Y, 8.0, -len)?;
    }
    if v.value.abs() >= 100.0 {
        let tip_y = (CENTER_Y - len).clamp(10.0, 350.0);
        let label = fmt_label!(12, "{}", libm::roundf(v.value / 50.0) as i64 * 50);
        scene.fill_color(palette::WHITE)?;
        scene.text(452.0, tip_y, 12.0, Anchor::CENTER, label.as_str())?;
    }
    Ok(())
}
