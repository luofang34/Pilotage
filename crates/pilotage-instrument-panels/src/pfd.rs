//! The Primary Flight Display: attitude ball, speed/altitude tapes, VSI,
//! and turn-rate cue, composed in fixed layers (background → attitude →
//! tapes → symbology → annunciation, ADR-0017).

use pilotage_instrument_scene::{PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{PanelData, SignalStatus};

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
/// `Horizon` is the phase-1 2D sky/ground ball. A synthetic-vision
/// background slots in here as a further variant carrying its viewport
/// and quality tier (reserved, ADR-0017) without touching the ladder,
/// tapes, or symbology layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundMode {
    /// Flat-shaded sky-over-ground attitude ball.
    #[default]
    Horizon,
}

/// PFD panel configuration.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PfdConfig {
    /// Attitude background selection.
    pub background: BackgroundMode,
    /// Speed-tape bands; `None` draws a bandless tape.
    pub v_speeds: Option<VSpeeds>,
}

/// Draws the PFD from resolved state.
pub fn draw_pfd(
    data: &PanelData,
    cfg: &PfdConfig,
    scene: &mut SceneWriter<'_>,
) -> Result<(), SceneError> {
    scene.fill_color(palette::BLACK)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;

    let att_status = data.roll_rad.status.worst(data.pitch_rad.status);
    if att_status.shows_value() {
        match cfg.background {
            BackgroundMode::Horizon => {
                horizon::draw_ball(scene, data.roll_rad.value, data.pitch_rad.value)?;
            }
        }
        horizon::draw_roll_scale(scene, data.roll_rad.value)?;
        horizon::draw_aircraft_symbol(scene)?;
        if att_status != SignalStatus::Valid {
            status_paint::draw_flag(scene, 240.0, 60.0, "ATT")?;
        }
    } else {
        status_paint::draw_red_x(scene, 110.0, 50.0, 260.0, 240.0, "ATT")?;
    }

    tapes::speed_tape(scene, data, cfg.v_speeds.as_ref())?;
    tapes::altitude_tape(scene, data)?;
    tapes::vsi(scene, data)?;
    draw_turn_rate(scene, data)?;
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
