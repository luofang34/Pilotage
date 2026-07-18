//! Speed and altitude tapes and the vertical-speed indicator.
//!
//! Tape scaling from the G5 proportions: speed ±25 kt over the panel
//! height (7.2 px/kt), altitude ±150 ft (1.2 px/ft), VSI ±1500 fpm full
//! scale.

use pilotage_instrument_scene::{
    Anchor, PaintMode, Rgba8, SceneError, SceneWriter, nominal_text_ink_width, nominal_text_width,
};
use pilotage_instrument_state::AltitudeClass;
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
    pointed_readout(scene, ias, text.as_str(), &IAS_READOUT)?;

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

/// Geometry of a pointed tape readout: the rectangular body spans
/// `far_x`..`near_x`, the tip at `tip_x` points toward the tape, and
/// the value is anchored at `text_x`, no larger than `preferred_size`.
struct PointedBox {
    far_x: f32,
    near_x: f32,
    tip_x: f32,
    text_x: f32,
    preferred_size: f32,
}

/// Airspeed readout: body at the panel's left edge, tip pointing right.
const IAS_READOUT: PointedBox = PointedBox {
    far_x: 2.0,
    near_x: 75.0,
    tip_x: 90.0,
    text_x: 40.0,
    preferred_size: 28.0,
};

/// Altitude readout: body at the panel's right edge, tip pointing left.
const ALT_READOUT: PointedBox = PointedBox {
    far_x: 478.0,
    near_x: 405.0,
    tip_x: 390.0,
    text_x: 442.0,
    preferred_size: 26.0,
};

/// Pointed value readout beside a tape. The run size shrinks
/// deterministically from `preferred_size` until the run's nominal ink
/// (the scene text-metrics contract every backend honors) fits the box
/// body, so a wide value — "10300", "-1030" — renders smaller, never
/// outside the box: an overflowing readout is silent display
/// corruption (DISP-02), which the box must make impossible for the
/// signal's whole representable range.
fn pointed_readout(
    scene: &mut SceneWriter<'_>,
    sig: Sig<f32>,
    text: &str,
    geo: &PointedBox,
) -> Result<(), SceneError> {
    scene.fill_color(palette::BOX_BG)?;
    let border = status_paint::status_accent(sig.status).unwrap_or(palette::WHITE);
    scene.stroke(border, 2.0)?;
    scene.polygon(
        PaintMode::FillStroke,
        &[
            [geo.far_x, 155.0],
            [geo.near_x, 155.0],
            [geo.near_x, 168.0],
            [geo.tip_x, 180.0],
            [geo.near_x, 192.0],
            [geo.near_x, 205.0],
            [geo.far_x, 205.0],
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
    let size = fitted_text_size(geo, shown.chars().count());
    scene.text(geo.text_x, 180.0, size, Anchor::CENTER, shown)?;
    Ok(())
}

/// Largest run size, capped at the box's preferred size, whose nominal
/// extents stay inside the box body from the box's text anchor: a
/// center anchor overhangs half the anchor width leftward and the ink
/// width minus that half rightward, and both extents scale linearly
/// with size, so the cap is a pure ratio.
fn fitted_text_size(geo: &PointedBox, chars: usize) -> f32 {
    let body_left = geo.far_x.min(geo.near_x);
    let body_right = geo.far_x.max(geo.near_x);
    let width = nominal_text_width(geo.preferred_size, chars);
    let ink = nominal_text_ink_width(geo.preferred_size, chars);
    let left_need = width / 2.0;
    let right_need = ink - width / 2.0;
    let mut scale = 1.0f32;
    if left_need > geo.text_x - body_left {
        scale = scale.min((geo.text_x - body_left) / left_need);
    }
    if right_need > body_right - geo.text_x {
        scale = scale.min((body_right - geo.text_x) / right_need);
    }
    geo.preferred_size * scale.max(0.0)
}

/// Right-edge altitude tape with selected-altitude bug and baro box.
/// The tape carries its reference label (REL amber, BARO/STD/MSL/AGL
/// white, RED for an unknown reference) so a local-relative height can
/// never read as barometric altitude; the bug and selection readout
/// render only when the selection's reference class matches.
pub fn altitude_tape(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let alt = data.altitude.value_ft;
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
        if let (true, Some(sel_m)) = (data.altitude.bug_compatible, data.selections.altitude_sel_m)
        {
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
    pointed_readout(scene, alt, text.as_str(), &ALT_READOUT)?;
    reference_label(scene, data)?;
    baro_and_sel_boxes(scene, data)?;
    Ok(())
}

/// The altitude reference label under the value box. REL is amber —
/// simulator-relative height demands attention — and an unknown wire
/// reference is red beside its failed tape.
fn reference_label(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let class = data.altitude.class;
    scene.fill_color(match class {
        AltitudeClass::LocalRelative => palette::AMBER,
        AltitudeClass::Unknown => palette::RED,
        _ => palette::WHITE,
    })?;
    scene.text(442.0, 222.0, 12.0, Anchor::CENTER, class.label())?;
    Ok(())
}

/// Setting and selection boxes. The setting readout states whether the
/// shown setting is applied: a barometric tape shows the applied value
/// in cyan, a pressure tape shows STD, and every other reference shows
/// the setting prefixed SET in grey — visibly not applied to the tape.
/// A selected/applied disagreement adds the amber BARO SEL flag.
fn baro_and_sel_boxes(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let baro = data.baro_hpa;
    let (text, color) = match data.altitude.class {
        AltitudeClass::BaroIndicated => (fmt_label!(12, "{:.0}", baro.value), palette::CYAN),
        AltitudeClass::Pressure => (fmt_label!(12, "STD"), palette::CYAN),
        _ => (fmt_label!(12, "SET {:.0}", baro.value), palette::GREY),
    };
    status_paint::readout_box(
        scene,
        390.0,
        335.0,
        90.0,
        25.0,
        text.as_str(),
        color,
        16.0,
        baro.status,
    )?;
    if data.altitude.setting_mismatch {
        status_paint::draw_flag(scene, 435.0, 330.0, "BARO SEL")?;
    }
    match (data.altitude.bug_compatible, data.selections.altitude_sel_m) {
        (true, Some(sel_m)) => {
            let sel_ft = sel_m * pilotage_instrument_state::units::M_TO_FT;
            let text = fmt_label!(12, "{}", libm::roundf(sel_ft) as i64);
            scene.fill_color(palette::BOX_BG)?;
            scene.stroke(palette::GREY, 1.5)?;
            scene.rect(PaintMode::FillStroke, 390.0, 0.0, 90.0, 24.0)?;
            scene.fill_color(palette::CYAN)?;
            scene.text(435.0, 12.0, 18.0, Anchor::CENTER, text.as_str())?;
        }
        (false, Some(_)) => {
            // A selection in an incompatible reference never renders as
            // a plausible number; the amber marker says why it is gone.
            scene.fill_color(palette::AMBER)?;
            scene.text(435.0, 12.0, 14.0, Anchor::CENTER, "SEL REF")?;
        }
        (_, None) => {}
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
