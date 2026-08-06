#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::BUILTIN_PANELS;
use pilotage_instrument_registry::{
    BackgroundCapability, DesignFrame, GroupSet, PanelDescriptor, Registry,
};

use super::admit;

#[test]
fn builtin_panels_pass_admission() {
    let registry = Registry::new(BUILTIN_PANELS).expect("composes");
    let report = admit(&registry).expect("shipped panels must be admissible");
    // PFD: (4 canonical + 3 extreme) states × (1 fed + 8 withheld);
    // HSI: (4 + 2) × 8; monitor: 5 × 2.
    assert_eq!(report.cases, 121);
    // Every warning is the PFD's groundspeed or baro readout: their
    // boxes are 90 units wide but a wide value at size 16 has ~107
    // units of nominal ink, so the run overhangs its box and the frame
    // edge (status_paint::readout_box draws at the requested size with
    // no fit shrink). Real display debt, honestly counted across every
    // corpus and extreme state; fixing the paint moves frame hashes and
    // is its own change. The ratchet makes any NEW unclipped off-frame
    // text a deliberate decision.
    assert_eq!(report.warnings.len(), 83);
    assert!(report.warnings.iter().all(|w| matches!(
        w,
        super::AdmissionWarning::FrameOverflow { panel: "pfd", text, .. }
            if text.starts_with("GS ") || text.starts_with("SET ")
    )));
}
mod background_checks;
mod provenance_checks;

/// One-panel descriptor around a draw fixture, shared by the
/// background and provenance fixture suites.
fn opaque_panel(draw: pilotage_instrument_registry::DrawFn) -> [PanelDescriptor; 1] {
    [PanelDescriptor {
        id: "probe",
        title: "Probe",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::EMPTY,
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::Opaque,
        config_schema: &[],
        group_regions: &[],
        extreme_states: &[],
        raster_baseline: None,
        draw,
    }]
}
