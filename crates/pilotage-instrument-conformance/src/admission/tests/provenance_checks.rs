//! Provenance-family fixtures: totality, claim testing under
//! withholding, claim bounding, and scanner-level structure.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_alerts::AlertOutput;
use pilotage_instrument_registry::{
    BackgroundCapability, ConfigBlob, DesignFrame, GroupSet, PanelDescriptor, PanelDrawError,
    Region, Registry,
};
use pilotage_instrument_scene::{Anchor, LayerId, Rgba8, SceneWriter};
use pilotage_instrument_state::{GroupId, PanelData};

use super::super::{AdmissionError, admit};
use super::opaque_panel;

/// A panel that draws a numeral carrying no provenance claim — the
/// totality hole that would otherwise escape every withholding case.
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
fn an_unclaimed_numeral_is_refused() {
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
        Err(AdmissionError::UntaggedNumeral {
            panel: "dishonest",
            ..
        })
    ));
}

/// A panel that fabricates an airspeed from other data only when the
/// air group is gone, claiming Air for the fake — clean when fed,
/// dishonest degradation. The claim is what convicts it: the run is
/// visible in the withhold-Air case while Air shows no value.
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
        scene.text_attributed(
            GroupId::Air.to_u8(),
            30.0,
            30.0,
            14.0,
            Anchor::CENTER,
            "999",
        )?;
    }
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_fabricated_claim_is_refused_when_its_group_is_withheld() {
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
        Err(AdmissionError::FabricatedNumeral {
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
        scene.text_attributed(
            GroupId::Air.to_u8(),
            -270.0,
            -270.0,
            14.0,
            Anchor::CENTER,
            "999",
        )?;
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
        Err(AdmissionError::FabricatedNumeral {
            panel: "translated",
            group: GroupId::Air,
            ..
        })
    ));
}
/// A claim on a group outside the panel's required set: the matrix
/// could never test it, so it is refused as structurally foreign.
fn draw_foreign_claim(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    scene.text_attributed(
        GroupId::Wind.to_u8(),
        30.0,
        30.0,
        14.0,
        Anchor::CENTER,
        "999",
    )?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_claim_outside_the_required_groups_is_refused() {
    let panels = std::boxed::Box::leak(std::boxed::Box::new(opaque_panel(draw_foreign_claim)));
    panels[0].background = BackgroundCapability::NotUsed;
    let registry = Registry::new(panels).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::ForeignClaim { tag, .. }) if tag == GroupId::Wind.to_u8()
    ));
}

/// A visible run claiming configuration provenance under the harness's
/// fixed empty configuration derives from nothing.
fn draw_config_claim(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    scene.text_attributed(
        pilotage_instrument_scene::ATTR_CONFIG,
        30.0,
        30.0,
        14.0,
        Anchor::CENTER,
        "V1 65",
    )?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_config_claim_is_refused_under_the_empty_config() {
    let panels = std::boxed::Box::leak(std::boxed::Box::new(opaque_panel(draw_config_claim)));
    panels[0].background = BackgroundCapability::NotUsed;
    let registry = Registry::new(panels).expect("structurally valid");
    assert!(matches!(
        admit(&registry),
        Err(AdmissionError::ConfigClaim { .. })
    ));
}

/// A tagged numeral scrolled fully outside its clip paints nothing, and
/// the claim rule is about what is shown — the panel passes.
fn draw_clipped_away(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(LayerId::Tapes)?;
    scene.save()?;
    scene.clip_rect(0.0, 0.0, 10.0, 10.0)?;
    scene.fill_color(Rgba8::rgb(255, 255, 255))?;
    scene.text_attributed(
        GroupId::Air.to_u8(),
        200.0,
        200.0,
        14.0,
        Anchor::CENTER,
        "999",
    )?;
    scene.restore()?;
    scene.end_layer(LayerId::Tapes)?;
    Ok(())
}

#[test]
fn a_claimed_numeral_clipped_fully_away_is_tolerated() {
    let panels = std::boxed::Box::leak(std::boxed::Box::new(opaque_panel(draw_clipped_away)));
    panels[0].background = BackgroundCapability::NotUsed;
    panels[0].required_groups = GroupSet::of(&[GroupId::Air]);
    let registry = Registry::new(panels).expect("structurally valid");
    admit(&registry).expect("an unshown claim is not a fabrication");
}

#[test]
fn a_dangling_claim_is_refused_by_the_scanner() {
    // Hand-encoded: version byte, then two stacked ATTRIBUTE commands —
    // the writer cannot produce this (text_attributed is atomic), so
    // the scanner is tested at the byte level.
    let bytes = [1u8, 0x31, 1, 0, 2, 0x31, 1, 0, 3];
    assert!(matches!(
        super::super::collect_runs(&bytes),
        Err(super::super::RunsDefect::MisplacedClaim)
    ));
    // A claim followed by a shape is equally malformed.
    let bytes = [
        1u8, 0x31, 1, 0, 2, 0x23, 17, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 0, 0, 128, 63,
    ];
    assert!(matches!(
        super::super::collect_runs(&bytes),
        Err(super::super::RunsDefect::MisplacedClaim)
    ));
}
