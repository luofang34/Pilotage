#![allow(clippy::expect_used, clippy::panic)]

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{LAYER_COUNT, SceneWriter};
use pilotage_instrument_state::{AircraftState, GroupId, PanelData};

use super::{Registry, RegistryError};
use crate::config::ConfigBlob;
use crate::descriptor::{
    BackgroundCapability, DesignFrame, ExtremeState, PanelDescriptor, PanelDrawError, Region,
};
use crate::group_set::GroupSet;

fn draw_nothing(
    _data: &PanelData,
    _config: &ConfigBlob<'_>,
    _alerts: Option<&AlertOutput>,
    _scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    Ok(())
}

fn nothing_fed() -> AircraftState {
    AircraftState::default()
}

const fn panel(id: &'static str) -> PanelDescriptor {
    PanelDescriptor {
        id,
        title: "Panel",
        required_layers: 0b0000_0110,
        required_groups: GroupSet::of(&[GroupId::Attitude, GroupId::Air]),
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
fn a_valid_composition_is_accepted_and_queryable() {
    static PANELS: [PanelDescriptor; 2] = [panel("alpha"), panel("beta-2")];
    let registry = Registry::new(&PANELS).expect("two well-formed panels");
    assert_eq!(registry.panels().len(), 2);
    assert_eq!(registry.by_id("beta-2").expect("registered").id, "beta-2");
    assert!(registry.by_id("gamma").is_none());
}

#[test]
fn an_empty_composition_is_refused() {
    assert_eq!(Registry::new(&[]).map(|_| ()), Err(RegistryError::Empty));
}

#[test]
fn malformed_and_duplicate_ids_are_refused() {
    static UPPER: [PanelDescriptor; 1] = [panel("PFD")];
    assert_eq!(
        Registry::new(&UPPER).map(|_| ()),
        Err(RegistryError::BadId { index: 0 })
    );
    static DUP: [PanelDescriptor; 2] = [panel("pfd"), panel("pfd")];
    assert_eq!(
        Registry::new(&DUP).map(|_| ()),
        Err(RegistryError::DuplicateId { index: 1 })
    );
}

#[test]
fn layer_mask_abuse_is_refused() {
    static NONE: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.required_layers = 0;
        p
    }];
    assert_eq!(
        Registry::new(&NONE).map(|_| ()),
        Err(RegistryError::NoRequiredLayers { index: 0 })
    );
    static BEYOND: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.required_layers = 1 << LAYER_COUNT;
        p
    }];
    assert_eq!(
        Registry::new(&BEYOND).map(|_| ()),
        Err(RegistryError::UndefinedLayerBits {
            index: 0,
            bits: 1 << LAYER_COUNT,
        })
    );
}

#[test]
fn a_degenerate_design_frame_is_refused() {
    static FLAT: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.design_frame = DesignFrame {
            width: 480.0,
            height: 0.0,
        };
        p
    }];
    assert_eq!(
        Registry::new(&FLAT).map(|_| ()),
        Err(RegistryError::BadDesignFrame { index: 0 })
    );
}

#[test]
fn schema_key_order_is_enforced() {
    use crate::config::ConfigKey;
    static UNSORTED: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.config_schema = &[ConfigKey(2), ConfigKey(1)];
        p
    }];
    assert_eq!(
        Registry::new(&UNSORTED).map(|_| ()),
        Err(RegistryError::SchemaKeysNotAscending { index: 0, key: 1 })
    );
}

#[test]
fn group_regions_must_stay_honest() {
    static FOREIGN: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.group_regions = &[(
            GroupId::Nav,
            Region {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        )];
        p
    }];
    assert_eq!(
        Registry::new(&FOREIGN).map(|_| ()),
        Err(RegistryError::RegionGroupNotRequired {
            index: 0,
            group: GroupId::Nav as u8,
        })
    );
    static OUTSIDE: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.group_regions = &[(
            GroupId::Attitude,
            Region {
                x: 470.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
        )];
        p
    }];
    assert_eq!(
        Registry::new(&OUTSIDE).map(|_| ()),
        Err(RegistryError::RegionOutsideFrame {
            index: 0,
            group: GroupId::Attitude as u8,
        })
    );
}

#[test]
fn extreme_state_ids_must_be_unique_and_well_formed() {
    static DUP: [PanelDescriptor; 1] = [{
        let mut p = panel("pfd");
        p.extreme_states = &[
            ExtremeState {
                id: "unusual-nose-high",
                build: nothing_fed,
            },
            ExtremeState {
                id: "unusual-nose-high",
                build: nothing_fed,
            },
        ];
        p
    }];
    assert_eq!(
        Registry::new(&DUP).map(|_| ()),
        Err(RegistryError::DuplicateExtremeId {
            index: 0,
            position: 1,
        })
    );
}
