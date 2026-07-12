//! The Primary Flight Display: attitude ball, speed/altitude tapes, VSI,
//! and turn-rate cue, composed in fixed layers (background → attitude →
//! tapes → symbology → annunciation, ADR-0017).

use pilotage_instrument_scene::{LayerId, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{ChevronSense, PanelData, SignalStatus};

use crate::palette;
use crate::status_paint;
use crate::{PANEL_H, PANEL_W};

mod horizon;
mod tapes;

/// Airframe reference speeds (knots) driving the speed-tape color bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VSpeeds {
    /// Stall, landing configuration (bottom of white arc).
    pub vs0_kt: f32,
    /// Stall, clean (bottom of green arc).
    pub vs_kt: f32,
    /// Maximum flap-extended (top of white arc).
    pub vfe_kt: f32,
    /// Maximum structural cruise (top of green arc).
    pub vno_kt: f32,
    /// Never exceed (red line).
    pub vne_kt: f32,
}

/// What fills the attitude background.
///
/// `Horizon` is the 2D sky/ground fill. `None` emits no
/// background layer at all: the safety compositor owns that band (a
/// hypothetical SVS raster composes strictly below the critical overlay),
/// and the layers above it are byte-identical either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundMode {
    /// Flat-shaded sky-over-ground attitude ball.
    #[default]
    Horizon,
    /// No background layer; the compositor supplies that band.
    None,
}

/// PFD panel configuration.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PfdConfig {
    /// Attitude background selection.
    pub background: BackgroundMode,
    /// Speed-tape bands; `None` draws a bandless tape.
    pub v_speeds: Option<VSpeeds>,
}

/// Draws the PFD from resolved state in the scene-layer bands:
/// optional background, then attitude symbology, tapes, and
/// annunciations, in ascending z-order. The layers above `Background`
/// never depend on the background mode, so the critical overlay stays
/// complete — byte-identical — when the background is absent.
/// The one declutter priority table (ATT-01): entering the unusual tier
/// removes exactly these elements. Primary attitude, the airspeed and
/// altitude tapes, VSI, and every failure flag/annunciation are never on
/// this list — declutter can only ever *add* attention to the horizon.
///
/// - minor (2.5° and 5°) pitch-ladder rows — major 10° bars remain
/// - speed-tape color bands
/// - the turn-rate cue
pub fn draw_pfd(
    data: &PanelData,
    cfg: &PfdConfig,
    scene: &mut SceneWriter<'_>,
) -> Result<(), SceneError> {
    let att_status = data.roll_rad.status.worst(data.pitch_rad.status);
    let declutter = att_status.shows_value() && data.presentation.unusual;

    match cfg.background {
        BackgroundMode::Horizon => {
            scene.begin_layer(LayerId::Background)?;
            scene.fill_color(palette::BLACK)?;
            scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;
            if att_status.shows_value() {
                horizon::draw_background(scene, data.roll_rad.value, data.pitch_rad.value)?;
            }
            scene.end_layer(LayerId::Background)?;
        }
        BackgroundMode::None => {}
    }

    scene.begin_layer(LayerId::Attitude)?;
    if att_status.shows_value() {
        horizon::draw_horizon_cues(scene, data.roll_rad.value, data.pitch_rad.value, declutter)?;
        horizon::draw_roll_scale(scene, data.roll_rad.value)?;
        horizon::draw_aircraft_symbol(scene)?;
        if let Some(sense) = data.presentation.chevrons {
            draw_recovery_chevrons(scene, data.roll_rad.value, sense)?;
        }
    }
    scene.end_layer(LayerId::Attitude)?;

    scene.begin_layer(LayerId::Tapes)?;
    tapes::speed_tape(
        scene,
        data,
        if declutter {
            None
        } else {
            cfg.v_speeds.as_ref()
        },
    )?;
    tapes::altitude_tape(scene, data)?;
    tapes::vsi(scene, data)?;
    if !declutter {
        draw_turn_rate(scene, data)?;
    }
    scene.end_layer(LayerId::Tapes)?;

    scene.begin_layer(LayerId::Annunciation)?;
    if att_status.shows_value() {
        if att_status != SignalStatus::Valid {
            status_paint::draw_flag(scene, 240.0, 60.0, "ATT")?;
        }
    } else {
        status_paint::draw_red_x(scene, 110.0, 50.0, 260.0, 240.0, "ATT")?;
    }
    if data.ias_kt.status == SignalStatus::Failed {
        status_paint::draw_red_x(scene, 8.0, 60.0, 74.0, 200.0, "IAS")?;
    }
    if data.alt_ft.status == SignalStatus::Failed {
        status_paint::draw_red_x(scene, 398.0, 60.0, 74.0, 200.0, "ALT")?;
    }
    scene.end_layer(LayerId::Annunciation)?;
    Ok(())
}

/// Recovery chevrons in the roll-rotated attitude frame, pointing toward
/// the horizon (an orientation cue, never a flight-director command).
/// Nose high puts the horizon below the aircraft symbol, so the chevrons
/// sit above center with their apexes downward; nose low mirrors it.
fn draw_recovery_chevrons(
    scene: &mut SceneWriter<'_>,
    roll_rad: f32,
    sense: ChevronSense,
) -> Result<(), SceneError> {
    scene.save()?;
    scene.translate(240.0, 180.0)?;
    scene.rotate(-roll_rad)?;
    scene.stroke(palette::RED, 6.0)?;
    let toward: f32 = match sense {
        ChevronSense::HorizonBelow => 1.0,
        ChevronSense::HorizonAbove => -1.0,
    };
    for offset in [56.0f32, 84.0] {
        let base_y = -toward * offset;
        let apex_y = base_y + toward * 22.0;
        scene.polyline(&[[-42.0, base_y], [0.0, apex_y], [42.0, base_y]])?;
    }
    scene.restore()?;
    Ok(())
}

/// Standard-rate turn is 3°/s, drawn at the ±62 px reference ticks.
fn draw_turn_rate(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let y = 340.0;
    scene.stroke(palette::WHITE, 2.0)?;
    scene.line(178.0, y - 6.0, 178.0, y + 6.0)?;
    scene.line(302.0, y - 6.0, 302.0, y + 6.0)?;
    scene.line(240.0, y - 4.0, 240.0, y + 4.0)?;
    if !data.turn_rate_rps.status.shows_value() {
        return Ok(());
    }
    let dps = data.turn_rate_rps.value * pilotage_instrument_state::units::RAD_TO_DEG;
    let len = (dps / 3.0 * 62.0).clamp(-73.0, 73.0);
    scene.fill_color(palette::MAGENTA)?;
    if len >= 0.0 {
        scene.rect(PaintMode::Fill, 240.0, y - 3.0, len, 6.0)?;
    } else {
        scene.rect(PaintMode::Fill, 240.0 + len, y - 3.0, -len, 6.0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
