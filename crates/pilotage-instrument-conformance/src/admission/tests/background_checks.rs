//! Background-contract fixtures: both defect directions and the
//! coverage evasions the graphics-state scanner refuses. The scanner
//! mirrors the real state machine: each evasion here was a verified
//! false admission (or false refusal) of an anchor-only scan, pinned
//! so none can return.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_alerts::AlertOutput;
use pilotage_instrument_registry::ConfigBlob;
use pilotage_instrument_registry::{
    BackgroundCapability, DesignFrame, GroupSet, PanelDescriptor, PanelDrawError, Registry,
};
use pilotage_instrument_scene::{LayerId, Rgba8, SceneWriter};
use pilotage_instrument_state::PanelData;

use super::super::{AdmissionError, admit};
use super::opaque_panel;

/// The shipped defect class: declaring NotUsed while painting an opaque
/// ground in the Background band. Human review caught this once; the
/// harness must catch it mechanically.
fn draw_notused_painter(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Background)?;
    scene.fill_color(Rgba8::rgb(0, 0, 0))?;
    scene.rect(
        pilotage_instrument_scene::PaintMode::Fill,
        0.0,
        0.0,
        480.0,
        360.0,
    )?;
    scene.end_layer(LayerId::Background)?;
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_notused_panel_that_paints_the_band_is_refused() {
    static DEFECT: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "shy-painter",
        title: "Shy Painter",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::EMPTY,
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_notused_painter,
    }];
    let registry = Registry::new(&DEFECT).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::BackgroundContract {
            panel: "shy-painter",
            declared: "NotUsed",
            ..
        })
    ));
}

/// The other direction: declaring Opaque out of optimism while painting
/// nothing in the band — a compositor promised coverage gets holes.
fn draw_optimist(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn an_opaque_panel_that_covers_nothing_is_refused() {
    static DEFECT: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "optimist",
        title: "Optimist",
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
        draw: draw_optimist,
    }];
    let registry = Registry::new(&DEFECT).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::BackgroundContract {
            panel: "optimist",
            declared: "Opaque",
            ..
        })
    ));
}
fn draw_clip_evasion(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Background)?;
    scene.save()?;
    scene.clip_rect(0.0, 0.0, 4.0, 4.0)?;
    scene.fill_color(Rgba8::rgb(10, 20, 30))?;
    scene.rect(
        pilotage_instrument_scene::PaintMode::Fill,
        0.0,
        0.0,
        480.0,
        360.0,
    )?;
    scene.restore()?;
    scene.end_layer(LayerId::Background)?;
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

fn draw_rotate_evasion(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Background)?;
    scene.save()?;
    scene.translate(240.0, 180.0)?;
    scene.rotate(core::f32::consts::FRAC_PI_4)?;
    scene.fill_color(Rgba8::rgb(10, 20, 30))?;
    scene.rect(
        pilotage_instrument_scene::PaintMode::Fill,
        -280.0,
        -280.0,
        560.0,
        560.0,
    )?;
    scene.restore()?;
    scene.end_layer(LayerId::Background)?;
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

fn draw_alpha_evasion(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Background)?;
    scene.fill_color(Rgba8::rgba(10, 20, 30, 8))?;
    scene.save()?;
    scene.fill_color(Rgba8::rgb(10, 20, 30))?;
    scene.restore()?;
    // The restore returned the paint state to alpha 8.
    scene.rect(
        pilotage_instrument_scene::PaintMode::Fill,
        0.0,
        0.0,
        480.0,
        360.0,
    )?;
    scene.end_layer(LayerId::Background)?;
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

fn draw_empty_band_notused(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Background)?;
    scene.end_layer(LayerId::Background)?;
    scene.begin_layer(LayerId::Tapes)?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn coverage_evasions_are_refused() {
    for (name, draw) in [
        (
            "clip",
            draw_clip_evasion as pilotage_instrument_registry::DrawFn,
        ),
        ("rotate", draw_rotate_evasion),
        ("alpha", draw_alpha_evasion),
    ] {
        let panels = std::boxed::Box::leak(std::boxed::Box::new(opaque_panel(draw)));
        let registry = Registry::new(panels).expect("structurally valid");
        assert!(
            matches!(
                admit(&registry),
                Err(AdmissionError::BackgroundContract {
                    declared: "Opaque",
                    ..
                })
            ),
            "{name} evasion must be refused"
        );
    }
}

#[test]
fn an_empty_band_under_notused_is_tolerated() {
    static SHY: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "shy",
        title: "Shy",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::EMPTY,
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_empty_band_notused,
    }];
    let registry = Registry::new(&SHY).expect("structurally valid");
    admit(&registry).expect("an opened-empty band paints nothing");
}

/// A panel that never emits a required band fails the layer family.
fn draw_empty(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    _scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    Ok(())
}

#[test]
fn a_panel_missing_its_required_band_is_refused() {
    static HOLLOW: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "hollow",
        title: "Hollow",
        required_layers: 1 << 4, // Annunciation
        required_groups: GroupSet::EMPTY,
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_empty,
    }];
    let registry = Registry::new(&HOLLOW).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::MissingRequiredLayers {
            panel: "hollow",
            ..
        })
    ));
}
