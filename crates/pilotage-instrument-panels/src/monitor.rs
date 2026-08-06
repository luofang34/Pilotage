//! The machine-monitoring text panel (AIR-IN-014): the proof that a
//! panel is a registry entry, not a shell change.
//!
//! Advisory-only readout of the MONITOR_TEXT channel: a title row and
//! up to eight text lines. Honest status like every panel — `Missing`
//! renders dashes, `Stale`/`Degraded` an amber flag, `Failed` a red X;
//! text never renders from a channel the resolver flagged.

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{Anchor, LayerId, PaintMode, SceneError, SceneWriter};
use pilotage_instrument_state::{PanelData, SignalStatus};
use pilotage_instrument_symbology::{annunciation, palette, safety, status_paint};

use crate::{PANEL_H, PANEL_W};

const LINE_H: f32 = 36.0;
const TEXT_X: f32 = 24.0;
const FIRST_LINE_Y: f32 = 84.0;

/// Draws the monitor panel from resolved state. Layers: `Tapes` carries
/// the readout rows (the panel's primary band), `Annunciation` the
/// status flags, matching the failure semantics of the flight panels.
pub fn draw_monitor(
    data: &PanelData,
    alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), SceneError> {
    let channel = &data.monitor_text;

    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(palette::BLACK)?;
    scene.rect(PaintMode::Fill, 0.0, 0.0, PANEL_W, PANEL_H)?;
    scene.fill_color(palette::GREY)?;
    scene.text(TEXT_X, 36.0, 20.0, Anchor::MIDDLE_LEFT, "MONITOR")?;
    if channel.status.shows_value() {
        scene.fill_color(palette::WHITE)?;
        for (row, line) in channel.value.lines().iter().enumerate() {
            let y = FIRST_LINE_Y + row as f32 * LINE_H;
            scene.text(TEXT_X, y, 18.0, Anchor::MIDDLE_LEFT, line.as_str())?;
        }
    } else {
        scene.fill_color(safety::ANNUNCIATION_WHITE)?;
        scene.text(TEXT_X, FIRST_LINE_Y, 18.0, Anchor::MIDDLE_LEFT, "---")?;
    }
    scene.end_layer(LayerId::Tapes)?;

    scene.begin_layer(LayerId::Annunciation)?;
    match channel.status {
        SignalStatus::Failed => {
            status_paint::draw_red_x(scene, 0.0, 0.0, PANEL_W, PANEL_H, "MON")?;
        }
        SignalStatus::Stale | SignalStatus::Degraded => {
            status_paint::draw_flag(scene, PANEL_W - 60.0, 36.0, "MON")?;
        }
        SignalStatus::Missing | SignalStatus::Valid => {}
    }
    if let Some(alerts) = alerts {
        annunciation::draw_alert_stack(scene, alerts)?;
    }
    scene.end_layer(LayerId::Annunciation)?;
    Ok(())
}
