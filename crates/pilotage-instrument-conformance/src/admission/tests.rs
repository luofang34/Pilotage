#![allow(clippy::expect_used, clippy::panic)]

use pilotage_alerts::AlertOutput;
use pilotage_instrument_panels::BUILTIN_PANELS;
use pilotage_instrument_registry::{
    BackgroundCapability, ConfigBlob, DesignFrame, GroupSet, PanelDescriptor, PanelDrawError,
    Region, Registry,
};
use pilotage_instrument_scene::{Anchor, LayerId, Rgba8, SceneWriter};
use pilotage_instrument_state::{GroupId, PanelData};

use super::{AdmissionError, admit};

#[test]
fn builtin_panels_pass_admission() {
    let registry = Registry::new(BUILTIN_PANELS).expect("composes");
    let report = admit(&registry).expect("shipped panels must be admissible");
    // PFD and HSI: 4 canonical states × (1 fed + 7 withheld) each; the
    // monitor panel: 4 × (1 fed + 2 withheld).
    assert_eq!(report.cases, 76);
    // Every warning is the PFD's groundspeed or baro readout: their
    // boxes are 90 units wide but a wide value at size 16 has ~107
    // units of nominal ink, so the run overhangs its box and the frame
    // edge (status_paint::readout_box draws at the requested size with
    // no fit shrink). Real display debt, honestly counted; fixing the
    // paint moves frame hashes and is its own change. The ratchet makes
    // any NEW unclipped off-frame text a deliberate decision.
    assert_eq!(report.warnings.len(), 33);
    assert!(report.warnings.iter().all(|w| matches!(
        w,
        super::AdmissionWarning::FrameOverflow { panel: "pfd", text, .. }
            if text.starts_with("GS ") || text.starts_with("SET ")
    )));
}

/// A panel that fabricates a numeric readout in its air-data region no
/// matter what it was given — the exact dishonesty admission exists to
/// refuse.
fn draw_dishonest(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    scene.text(30.0, 30.0, 14.0, Anchor::CENTER, "999")?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn an_always_fabricating_panel_is_refused_at_the_furniture_stage() {
    static DISHONEST: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "dishonest",
        title: "Dishonest",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::of(&[GroupId::Air]),
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[(
            GroupId::Air,
            Region {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        )],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_dishonest,
    }];
    let registry = Registry::new(&DISHONEST).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::DishonestFurniture {
            panel: "dishonest",
            group: GroupId::Air,
            ..
        })
    ));
}

/// A panel that fabricates an airspeed from other data only when the
/// air group is gone — clean furniture, dishonest degradation.
fn draw_leaking(
    data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    let air_missing = !data.ias_kt.status.shows_value();
    let kinematics_present = data.gs_kt.status.shows_value();
    if air_missing && kinematics_present {
        scene.text(30.0, 30.0, 14.0, Anchor::CENTER, "999")?;
    }
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_panel_leaking_numbers_into_a_withheld_region_is_refused() {
    static LEAKING: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "leaking",
        title: "Leaking",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::of(&[GroupId::Air, GroupId::Kinematics]),
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[(
            GroupId::Air,
            Region {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        )],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_leaking,
    }];
    let registry = Registry::new(&LEAKING).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::DishonestNumeral {
            panel: "leaking",
            group: GroupId::Air,
            ..
        })
    ));
}

/// The same fabricator hiding behind a transform: the checks work in
/// design space, so a translate cannot move a run out of their sight.
fn draw_translated(
    data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    let air_missing = !data.ias_kt.status.shows_value();
    let kinematics_present = data.gs_kt.status.shows_value();
    if air_missing && kinematics_present {
        scene.save()?;
        scene.translate(300.0, 300.0)?;
        scene.text(-270.0, -270.0, 14.0, Anchor::CENTER, "999")?;
        scene.restore()?;
    }
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_transform_cannot_hide_a_dishonest_numeral() {
    static TRANSLATED: [PanelDescriptor; 1] = [PanelDescriptor {
        id: "translated",
        title: "Translated",
        required_layers: 1 << 2, // Tapes
        required_groups: GroupSet::of(&[GroupId::Air, GroupId::Kinematics]),
        design_frame: DesignFrame {
            width: 480.0,
            height: 360.0,
        },
        background: BackgroundCapability::NotUsed,
        config_schema: &[],
        group_regions: &[(
            GroupId::Air,
            Region {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        )],
        extreme_states: &[],
        raster_baseline: None,
        draw: draw_translated,
    }];
    let registry = Registry::new(&TRANSLATED).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::DishonestNumeral {
            panel: "translated",
            group: GroupId::Air,
            ..
        })
    ));
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
