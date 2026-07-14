//! SRC-01 airspeed source annunciation.
//!
//! The selected source's value and its id are read from one inseparable
//! [`SourcedFunction`] result, so the readout number and its `SRC` label can
//! never name different sources, and on reversion both switch together. A
//! sustained miscompare or a non-primary selection is drawn amber.

use pilotage_instrument_scene::{Anchor, SceneError, SceneWriter};
use pilotage_instrument_state::{ComparisonState, SourcedFunction};

use crate::fixed_str::fmt_label;
use crate::palette;

/// Draws the airspeed source readout from the inseparable value+source
/// result. Draws nothing when no source is selected (unmonitored, or every
/// candidate failed — the tape's own failure flag covers that case).
pub(crate) fn draw_source_readout(
    scene: &mut SceneWriter<'_>,
    airspeed: &SourcedFunction<f32>,
) -> Result<(), SceneError> {
    let Some(sourced) = airspeed.selected else {
        return Ok(());
    };
    let miscompare = airspeed.state == ComparisonState::Miscompare;
    let color = if miscompare || airspeed.reverted {
        palette::AMBER
    } else {
        palette::GREEN
    };
    scene.fill_color(color)?;
    let value = fmt_label!(8, "{}", libm::roundf(sourced.value()) as i32);
    scene.text(45.0, 210.0, 16.0, Anchor::CENTER, value.as_str())?;
    let label = fmt_label!(8, "SRC{}", sourced.source().0);
    scene.text(45.0, 226.0, 12.0, Anchor::CENTER, label.as_str())?;
    if miscompare {
        scene.text(45.0, 240.0, 12.0, Anchor::CENTER, "IAS CMP")?;
    }
    Ok(())
}
