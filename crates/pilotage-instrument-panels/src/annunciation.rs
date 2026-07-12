//! Manager-driven alert annunciation shared by every panel.
//!
//! Each panel passes the SAME [`AlertOutput`] snapshot into
//! [`draw_alert_stack`], so a semantic alert difference between panels is
//! structurally impossible: the stack is a pure function of the manager
//! output, and the caller supplies one snapshot per frame. Primary-data
//! failure flags (the red X and ATT/IAS/ALT/HDG flags) are drawn by each
//! panel directly from `PanelData` and never pass through here — the
//! alerting path can be absent, saturated, or faulted without touching
//! them.

use pilotage_alerts::{
    AlertClass, AlertCondition, AlertId, AlertOutput, AltFault, DisplayFault, DynFault,
    ManagerHealth, MiscompareFault, NavFault, SystemNote,
};
use pilotage_instrument_scene::{Anchor, Rgba8, SceneError, SceneWriter};

use crate::palette;

const STACK_X: f32 = 100.0;
const STACK_BASE_Y: f32 = 352.0;
const ROW_STEP: f32 = 16.0;
/// Visible rows; anything beyond collapses into the MORE marker so the
/// stack can never crowd primary symbology.
const STACK_ROWS: usize = 3;

fn class_color(class: AlertClass) -> Rgba8 {
    match class {
        AlertClass::Warning => palette::RED,
        AlertClass::Caution => palette::AMBER,
        AlertClass::Advisory | AlertClass::Status | AlertClass::Maintenance => palette::WHITE,
    }
}

/// Short, glyph-pack-covered label for a stable alert identity. An
/// identity outside the known vocabulary still shows — as the generic
/// ALERT token — because an unknown fault must never be invisible.
fn alert_label(id: AlertId) -> &'static str {
    use AlertCondition as C;
    let table: [(AlertId, &'static str); 20] = [
        (C::Altitude(AltFault::ReferenceLost).id(), "ALT REF"),
        (C::Altitude(AltFault::DatumMiscompare).id(), "BARO CMP"),
        (C::Altitude(AltFault::Unavailable).id(), "ALT SRC"),
        (C::Heading(NavFault::HeadingReferenceLost).id(), "HDG REF"),
        (C::Heading(NavFault::CourseSourceInvalid).id(), "CRS SRC"),
        (C::Heading(NavFault::Unavailable).id(), "NAV SRC"),
        (C::TurnSlip(DynFault::TurnRateInvalid).id(), "TRN RATE"),
        (C::TurnSlip(DynFault::SlipInvalid).id(), "SLIP"),
        (C::TurnSlip(DynFault::Unavailable).id(), "TRN SRC"),
        (C::Miscompare(MiscompareFault::Attitude).id(), "ATT CMP"),
        (C::Miscompare(MiscompareFault::Airspeed).id(), "IAS CMP"),
        (C::Miscompare(MiscompareFault::Altitude).id(), "ALT CMP"),
        (C::Miscompare(MiscompareFault::Heading).id(), "HDG CMP"),
        (C::Display(DisplayFault::RendererStalled).id(), "DSP STALL"),
        (
            C::Display(DisplayFault::FrameGenerationLost).id(),
            "DSP GEN",
        ),
        (
            C::Display(DisplayFault::CommandBufferCorrupt).id(),
            "DSP BUF",
        ),
        (C::Display(DisplayFault::BackendLost).id(), "DSP LOST"),
        (C::Display(DisplayFault::RetainedImage).id(), "DSP HOLD"),
        (C::System(SystemNote::DatabaseStale).id(), "DB OLD"),
        (C::System(SystemNote::MaintenanceRequired).id(), "MAINT"),
    ];
    for (known, label) in table {
        if known == id {
            return label;
        }
    }
    if (C::System(SystemNote::ConfigMismatch).id()) == id {
        return "CONFIG";
    }
    let code = (id.0 & 0xff) as u8;
    if (C::FrameMismatch { code }).id() == id {
        return "FRAME";
    }
    "ALERT"
}

/// Draws the manager's alert stack in the annunciation layer:
/// priority-ordered rows (the manager's own ordering), warning red,
/// caution amber, everything lower white. Inhibited and decluttered
/// alerts are hidden here exactly as the manager flagged them; a
/// truncated or overflowed list shows the amber MORE marker, and a
/// faulted alerting path shows ALRT FAIL — the degradation itself is
/// annunciated from the primary render path.
pub(crate) fn draw_alert_stack(
    scene: &mut SceneWriter<'_>,
    alerts: &AlertOutput,
) -> Result<(), SceneError> {
    if alerts.health() == ManagerHealth::Faulted {
        scene.fill_color(palette::AMBER)?;
        scene.text(
            STACK_X,
            STACK_BASE_Y - 4.0 * ROW_STEP,
            12.0,
            Anchor::BASELINE_LEFT,
            "ALRT FAIL",
        )?;
    }
    let mut row = 0usize;
    let mut truncated = alerts.overflow();
    for alert in alerts.active() {
        if alert.inhibited || alert.decluttered {
            continue;
        }
        if row >= STACK_ROWS {
            truncated = true;
            break;
        }
        scene.fill_color(class_color(alert.class))?;
        scene.text(
            STACK_X,
            STACK_BASE_Y - row as f32 * ROW_STEP,
            12.0,
            Anchor::BASELINE_LEFT,
            alert_label(alert.id),
        )?;
        row = row.wrapping_add(1);
    }
    if truncated {
        scene.fill_color(palette::AMBER)?;
        scene.text(
            STACK_X,
            STACK_BASE_Y - (row as f32) * ROW_STEP,
            12.0,
            Anchor::BASELINE_LEFT,
            "MORE",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
