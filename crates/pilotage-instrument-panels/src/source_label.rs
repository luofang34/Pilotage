//! Per-function source annunciation shared by the PFD and HSI (SRC-01).
//!
//! The authoritative value a tape draws is already the selected source's own
//! (the resolve path binds it), so this only names that source and flags a
//! non-primary selection or a sustained miscompare. The label reads the same
//! [`SourcedFunction`] the value came from, so the number and its label can
//! never name different sources.

use pilotage_instrument_scene::{Anchor, SceneError, SceneWriter};
use pilotage_instrument_state::{ComparisonState, SourcedFunction};

use crate::fixed_str::fmt_label;
use crate::palette;

/// Draws the selected source id under a function prefix (e.g. `IAS2`), amber
/// on a reversion or a sustained miscompare, with a `{prefix} CMP` cue on a
/// miscompare. Draws nothing when the function is unmonitored or every
/// candidate failed — the instrument's own failure flag covers that.
pub(crate) fn draw_source_label<T: Copy>(
    scene: &mut SceneWriter<'_>,
    x: f32,
    y: f32,
    prefix: &str,
    selection: &SourcedFunction<T>,
) -> Result<(), SceneError> {
    let Some(source) = selection.selected else {
        return Ok(());
    };
    let miscompare = selection.state == ComparisonState::Miscompare;
    let color = if miscompare || selection.reverted {
        palette::AMBER
    } else {
        palette::GREEN
    };
    scene.fill_color(color)?;
    let label = fmt_label!(12, "{prefix}{}", source.source().0);
    scene.text(x, y, 12.0, Anchor::CENTER, label.as_str())?;
    if miscompare {
        let cmp = fmt_label!(12, "{prefix} CMP");
        scene.text(x, y + 12.0, 12.0, Anchor::CENTER, cmp.as_str())?;
    }
    Ok(())
}
