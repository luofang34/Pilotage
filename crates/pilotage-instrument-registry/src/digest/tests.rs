#![allow(clippy::expect_used, clippy::panic)]

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::PanelData;

use super::{DigestError, scene_digest};
use crate::config::ConfigBlob;
use crate::descriptor::{BackgroundCapability, DesignFrame, PanelDescriptor, PanelDrawError};
use crate::group_set::GroupSet;
use crate::registry::Registry;

fn draw_nothing(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    _scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    Ok(())
}

const fn panel(id: &'static str) -> PanelDescriptor {
    PanelDescriptor {
        id,
        title: "Panel",
        required_layers: 0b10,
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
        draw: draw_nothing,
    }
}

#[test]
fn the_digest_binds_panel_identity_and_composition() {
    static ONE: [PanelDescriptor; 1] = [panel("alpha")];
    static RENAMED: [PanelDescriptor; 1] = [panel("beta")];
    static TWO: [PanelDescriptor; 2] = [panel("alpha"), panel("beta")];
    let mut scratch = std::vec![0u8; MAX_SCENE_BYTES];
    let digest = |panels: &'static [PanelDescriptor], scratch: &mut [u8]| {
        scene_digest(&Registry::new(panels).expect("composes"), scratch).expect("digests")
    };
    let one = digest(&ONE, &mut scratch);
    assert_ne!(one, digest(&RENAMED, &mut scratch), "panel id is bound");
    assert_ne!(one, digest(&TWO, &mut scratch), "composition is bound");
}

fn draw_one_layer(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    scene.begin_layer(pilotage_instrument_scene::LayerId::Attitude)?;
    scene.end_layer(pilotage_instrument_scene::LayerId::Attitude)?;
    Ok(())
}

#[test]
fn an_undersized_scratch_fails_typed_not_truncated() {
    static ONE: [PanelDescriptor; 1] = [{
        let mut p = panel("alpha");
        p.draw = draw_one_layer;
        p
    }];
    let registry = Registry::new(&ONE).expect("composes");
    // Too small for the writer's own header: refused before any draw.
    let mut none = [0u8; 0];
    assert!(matches!(
        scene_digest(&registry, &mut none),
        Err(DigestError::Scratch { len: 0 })
    ));
    // Big enough to open the writer, too small for the panel's layer:
    // the panel's own refusal, with the panel and state named.
    let mut tiny = [0u8; 2];
    assert!(matches!(
        scene_digest(&registry, &mut tiny),
        Err(DigestError::Draw { panel: "alpha", .. })
    ));
}
