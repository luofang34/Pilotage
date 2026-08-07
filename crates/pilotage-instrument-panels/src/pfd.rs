//! The Primary Flight Display: attitude ball, speed/altitude tapes, VSI,
//! and turn-rate cue, composed in fixed layers (background → attitude →
//! tapes → symbology → annunciation, ADR-0017).

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{Anchor, LayerId, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{ChevronSense, FdEngagement, GroupId, PanelData, SignalStatus};

use pilotage_instrument_symbology::{annunciation, palette, safety, source_label, status_paint};

use crate::{PANEL_H, PANEL_W};

mod horizon;
mod panel_config;
mod tapes;

pub use panel_config::PFD_CONFIG_SCHEMA;

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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundMode {
    /// Flat-shaded sky-over-ground attitude ball.
    #[default]
    Horizon,
    /// No background layer; the compositor supplies that band.
    None,
    /// Synthetic-vision imagery in the background band, accept-and-cede
    /// (ADR-0033): the panel validates and carries the request but
    /// emits exactly what [`BackgroundMode::None`] emits — the band is
    /// ceded to whatever renders the imagery, and the critical overlay
    /// above it never depends on the choice.
    Svs {
        /// Viewport within the design frame the imagery should fill.
        viewport: SvsViewport,
        /// Quality tier requested from the renderer.
        quality: u8,
    },
}

/// The design-frame rectangle synthetic vision should fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvsViewport {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
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
    alerts: Option<&AlertOutput>,
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
                horizon::draw_background(
                    scene,
                    data.roll_rad.value,
                    data.pitch_rad.value,
                    data.presentation.min_reverse_band_rad,
                )?;
            }
            scene.end_layer(LayerId::Background)?;
        }
        // Svs cedes the band exactly like None until the SVS renderer
        // exists; a guardrail test pins the equivalence.
        BackgroundMode::None | BackgroundMode::Svs { .. } => {}
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

    // Director command bars are guidance overlaying the attitude
    // field: layers encode once each in ascending z-order, so the
    // Guidance band paints over Attitude by the layer contract — an
    // explicit decision, not an accident of band choice (#261). The
    // band is always emitted so the layer contract holds under every
    // degradation; its content disappears with the director.
    scene.begin_layer(LayerId::Guidance)?;
    // The unusual-attitude tier strips everything that competes with
    // the horizon and the recovery chevrons — command bars included:
    // they overlay the exact chevron region from a higher band, and a
    // recovery is flown to the horizon, not to an autopilot command.
    if !declutter {
        director_bars(scene, data, att_status)?;
    }
    scene.end_layer(LayerId::Guidance)?;

    annunciation_band(scene, data, alerts, att_status)?;
    Ok(())
}

/// Dual-cue flight-director command bars, drawn only from a fully
/// valid, ENGAGED director over a shown attitude: the bars mean "fly
/// toward the command", so they need both the command and the current
/// attitude, and under any degradation they disappear entirely — a
/// frozen or dashed command is still a command (#261).
fn director_bars(
    scene: &mut SceneWriter<'_>,
    data: &PanelData,
    att_status: SignalStatus,
) -> Result<(), SceneError> {
    let fd = data.director;
    if fd.engagement != FdEngagement::Engaged
        || !fd.status.shows_value()
        || !att_status.shows_value()
    {
        return Ok(());
    }
    const PX_PER_DEG_ROLL: f32 = 3.0;
    use horizon::PX_PER_DEG_PITCH;
    const RAD_TO_DEG: f32 = pilotage_instrument_state::units::RAD_TO_DEG;
    let pitch_err_px = ((fd.pitch_cmd_rad - data.pitch_rad.value) * RAD_TO_DEG * PX_PER_DEG_PITCH)
        .clamp(-90.0, 90.0);
    let roll_err_px =
        ((fd.roll_cmd_rad - data.roll_rad.value) * RAD_TO_DEG * PX_PER_DEG_ROLL).clamp(-90.0, 90.0);
    scene.fill_color(palette::MAGENTA)?;
    // Fly-to bars: the pitch bar sits where the commanded pitch is
    // (positive command error displaces it up), the roll bar likewise
    // sideways; centering both on the aircraft symbol satisfies them.
    scene.rect(
        PaintMode::Fill,
        240.0 - 45.0,
        180.0 - pitch_err_px - 2.5,
        90.0,
        5.0,
    )?;
    scene.rect(
        PaintMode::Fill,
        240.0 + roll_err_px - 2.5,
        180.0 - 45.0,
        5.0,
        90.0,
    )?;
    Ok(())
}

/// The Annunciation band: attitude/airspeed/altitude failure flags,
/// per-function source labels, and the alert stack.
fn annunciation_band(
    scene: &mut SceneWriter<'_>,
    data: &PanelData,
    alerts: Option<&AlertOutput>,
    att_status: SignalStatus,
) -> Result<(), SceneError> {
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
    // The mode is part of the director feature, not a follow-up: a
    // command bar without the mode that produced it is not
    // interpretable. Annunciated when armed or engaged; claimed from
    // the director group so the withholding matrix tests it.
    if data.director.status.shows_value()
        && matches!(
            data.director.engagement,
            FdEngagement::Armed | FdEngagement::Engaged
        )
    {
        scene.fill_color(palette::MAGENTA)?;
        scene.text_attributed(
            GroupId::FlightDirector.to_u8(),
            240.0,
            44.0,
            12.0,
            Anchor::CENTER,
            data.director.mode.label(),
        )?;
    }
    if data.altitude.value_ft.status == SignalStatus::Failed {
        status_paint::draw_red_x(scene, 398.0, 60.0, 74.0, 200.0, "ALT")?;
    }
    source_label::draw_source_label(
        scene,
        GroupId::Air.to_u8(),
        45.0,
        250.0,
        "IAS",
        &data.sources.airspeed,
    )?;
    source_label::draw_source_label(
        scene,
        GroupId::Kinematics.to_u8(),
        435.0,
        250.0,
        "ALT",
        &data.sources.altitude,
    )?;
    source_label::draw_source_label(
        scene,
        GroupId::Attitude.to_u8(),
        240.0,
        300.0,
        "ATT",
        &data.sources.attitude,
    )?;
    if let Some(alerts) = alerts {
        annunciation::draw_alert_stack(scene, alerts)?;
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
    scene.stroke(safety::FAILURE_RED, 6.0)?;
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
/// Only the POINTER saturates at the scale edge (±73 px); the resolved
/// value stays unclamped for monitoring. The cue labels its basis, and
/// a required-but-unusable turn indication flags TRN instead of
/// quietly disappearing.
fn draw_turn_rate(scene: &mut SceneWriter<'_>, data: &PanelData) -> Result<(), SceneError> {
    let y = 340.0;
    scene.stroke(palette::WHITE, 2.0)?;
    scene.line(178.0, y - 6.0, 178.0, y + 6.0)?;
    scene.line(302.0, y - 6.0, 302.0, y + 6.0)?;
    scene.line(240.0, y - 4.0, 240.0, y + 4.0)?;
    let turn = &data.turn;
    if !turn.rate_rps.status.shows_value() {
        if data.require_dynamics_cue {
            status_paint::draw_flag(scene, 240.0, y - 12.0, "TRN")?;
        }
        return draw_slip_ball(scene, data, y);
    }
    let dps = turn.rate_rps.value * pilotage_instrument_state::units::RAD_TO_DEG;
    let len = (dps / 3.0 * 62.0).clamp(-73.0, 73.0);
    scene.fill_color(palette::MAGENTA)?;
    if len >= 0.0 {
        scene.rect(PaintMode::Fill, 240.0, y - 3.0, len, 6.0)?;
    } else {
        scene.rect(PaintMode::Fill, 240.0 + len, y - 3.0, -len, 6.0)?;
    }
    scene.fill_color(palette::WHITE)?;
    scene.text(
        310.0,
        y + 4.0,
        10.0,
        Anchor::BASELINE_LEFT,
        turn.basis.label(),
    )?;
    draw_slip_ball(scene, data, y)
}

/// Slip/skid ball under the turn cue. The ball displaces OPPOSITE the
/// lateral specific force (body +Y right ⇒ ball left), one bracket
/// width per 2 m/s²; the pointer clamps at ±1.5 widths while the
/// resolved value stays unclamped. A missing input draws brackets and
/// NO ball — a centered ball is a coordination claim nobody made — and
/// flags SLIP when the profile requires the cue.
fn draw_slip_ball(scene: &mut SceneWriter<'_>, data: &PanelData, y: f32) -> Result<(), SceneError> {
    let by = y + 14.0;
    scene.stroke(palette::WHITE, 1.5)?;
    scene.line(233.0, by - 5.0, 233.0, by + 5.0)?;
    scene.line(247.0, by - 5.0, 247.0, by + 5.0)?;
    let slip = data.slip_lat_mps2;
    if !slip.status.shows_value() {
        if data.require_dynamics_cue {
            status_paint::draw_flag(scene, 270.0, by, "SLIP")?;
        }
        return Ok(());
    }
    let dx = (-slip.value / 2.0 * 14.0).clamp(-21.0, 21.0);
    scene.fill_color(palette::WHITE)?;
    scene.circle(PaintMode::Fill, 240.0 + dx, by, 4.0)?;
    Ok(())
}

#[cfg(test)]
mod attitude_tests;
#[cfg(test)]
mod datum_tests;
#[cfg(test)]
mod director_tests;
#[cfg(test)]
mod dyn_tests;
#[cfg(test)]
mod source_tests;
#[cfg(test)]
pub(crate) mod tests;
