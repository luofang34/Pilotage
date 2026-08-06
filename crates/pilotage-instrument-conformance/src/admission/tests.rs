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
    // PFD: (4 canonical + 2 extreme) states × (1 fed + 7 withheld);
    // HSI: 5 × 8; monitor: 5 × 2.
    assert_eq!(report.cases, 98);
    // Every warning is the PFD's groundspeed or baro readout: their
    // boxes are 90 units wide but a wide value at size 16 has ~107
    // units of nominal ink, so the run overhangs its box and the frame
    // edge (status_paint::readout_box draws at the requested size with
    // no fit shrink). Real display debt, honestly counted across every
    // corpus and extreme state; fixing the paint moves frame hashes and
    // is its own change. The ratchet makes any NEW unclipped off-frame
    // text a deliberate decision.
    assert_eq!(report.warnings.len(), 59);
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

/// The scanner mirrors the real state machine: each of these was a
/// verified false admission (or false refusal) of an earlier
/// anchor-only scan, pinned here so none can return.
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
