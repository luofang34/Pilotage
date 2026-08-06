//! The Horizontal Situation Indicator: rotating compass rose, heading
//! bug, ground-track diamond, course deviation indicator, and data boxes.

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{Anchor, LayerId, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::HeadingReference;
use pilotage_instrument_state::{GroupId, NavSource, PanelData, RoseBasis, SignalStatus};

use pilotage_instrument_symbology::{annunciation, palette, safety, source_label, status_paint};

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
pub fn draw_hsi(
    data: &PanelData,
    alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), SceneError> {
    scene.begin_layer(LayerId::Background)?;
    scene.fill_color(palette::BLACK)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;
    scene.end_layer(LayerId::Background)?;

    // The rose basis is resolve's selection — the same one every
    // converted angular quantity was presented against, so the rose
    // and its quantities cannot disagree. Under a track basis the
    // rotating symbology derives from kinematics and claims it.
    let hdg = data.heading.value_rad;
    // Belt and suspenders: the basis names the selection, the signal's
    // own status still gates the paint — a basis that disagrees with
    // its signal draws nothing rather than a plausible rose.
    let up = match data.rose_basis {
        RoseBasis::Heading if hdg.status.shows_value() => Some((hdg.value, GroupId::Heading)),
        RoseBasis::Track if data.track_rad.status.shows_value() => {
            Some((data.track_rad.value, GroupId::Kinematics))
        }
        _ => None,
    };
    scene.begin_layer(LayerId::Attitude)?;
    if let Some((up_rad, group)) = up {
        rose::draw_rose(scene, group.to_u8(), up_rad)?;
        if data.heading_bug_rose_rad.status.shows_value() {
            rose::draw_heading_bug(scene, up_rad, data.heading_bug_rose_rad.value)?;
        }
        if data.track_rad.status.shows_value() {
            rose::draw_track_diamond(scene, up_rad, data.track_rad.value)?;
        }
    }
    scene.end_layer(LayerId::Attitude)?;

    scene.begin_layer(LayerId::Tapes)?;
    match up {
        Some((_, group)) if data.rose_basis == RoseBasis::Track => {
            rose::draw_heading_box(scene, group.to_u8(), data.track_rad)?;
        }
        _ => rose::draw_heading_box(scene, GroupId::Heading.to_u8(), hdg)?,
    }
    boxes::wind_box(scene, data)?;
    boxes::dist_box(scene, data)?;
    boxes::course_box(scene, data)?;
    boxes::heading_sel_box(scene, data)?;
    scene.end_layer(LayerId::Tapes)?;

    scene.begin_layer(LayerId::Guidance)?;
    if let Some((up_rad, _)) = up
        && data.nav.data.source != NavSource::None
        && data.nav.status.shows_value()
        && data.nav.course_rose_rad.status.shows_value()
    {
        cdi::draw_cdi(scene, &data.nav, up_rad)?;
    }
    boxes::vertical_deviation(scene, data)?;
    scene.end_layer(LayerId::Guidance)?;

    annunciation_band(scene, data, alerts)?;
    Ok(())
}

/// The Annunciation band: nav degradation flag, the rose-basis
/// reference annunciation (or the heading failure X), the heading
/// source label, and the alert stack.
fn annunciation_band(
    scene: &mut SceneWriter<'_>,
    data: &PanelData,
    alerts: Option<&AlertOutput>,
) -> Result<(), SceneError> {
    let hdg = data.heading.value_rad;
    scene.begin_layer(LayerId::Annunciation)?;
    if data.nav.data.source != NavSource::None
        && data.nav.status.shows_value()
        && data.nav.status != SignalStatus::Valid
    {
        status_paint::draw_flag(scene, CX, CY + 60.0, "NAV")?;
    }
    match data.rose_basis {
        RoseBasis::Heading if hdg.status.shows_value() => {
            scene.fill_color(match data.heading.reference {
                HeadingReference::SimLocalTrue => safety::CAUTION_AMBER,
                _ => palette::WHITE,
            })?;
            scene.text(
                CX,
                CY - 118.0,
                12.0,
                Anchor::CENTER,
                data.heading.reference.label(),
            )?;
        }
        // A track-up rose must never read as a heading: the reference
        // slot annunciates TRK in the selection color, distinct from
        // every heading reference label.
        RoseBasis::Track if data.track_rad.status.shows_value() => {
            scene.fill_color(palette::CYAN)?;
            scene.text(CX, CY - 118.0, 12.0, Anchor::CENTER, "TRK")?;
        }
        _ => {
            status_paint::draw_red_x(scene, CX - 140.0, CY - 140.0, 280.0, 280.0, "HDG")?;
        }
    }
    source_label::draw_source_label(
        scene,
        GroupId::Heading.to_u8(),
        CX + 90.0,
        14.0,
        "HDG",
        &data.sources.heading,
    )?;
    if let Some(alerts) = alerts {
        annunciation::draw_alert_stack(scene, alerts)?;
    }
    scene.end_layer(LayerId::Annunciation)?;
    Ok(())
}

#[cfg(test)]
mod source_tests;
#[cfg(test)]
mod tests;
