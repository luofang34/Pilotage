//! Built-in panel descriptors: the registry entries every shell
//! composes (ADR-0029, ADR-0033).
//!
//! The masks and identities that used to live as shell constants are
//! owned here, beside the panels they describe. `group_regions` and
//! `extreme_states` are populated when their consumers land (the
//! admission harness and the canonical-state corpus); `raster_baseline`
//! is populated when the pinned frame hashes travel into the
//! descriptors.

use pilotage_alerts::AlertOutput;
use pilotage_instrument_registry::{
    BackgroundCapability, ConfigBlob, DesignFrame, GroupSet, PanelDescriptor, PanelDrawError,
};
use pilotage_instrument_scene::{LayerId, SceneWriter};
use pilotage_instrument_state::{GroupId, PanelData};

use crate::pfd::PFD_CONFIG_SCHEMA;
use crate::{PANEL_H, PANEL_W, PfdConfig, draw_hsi, draw_pfd};

const fn layer_bit(layer: LayerId) -> u8 {
    1u8 << layer.to_u8()
}

fn draw_pfd_panel(
    data: &PanelData,
    config: &ConfigBlob<'_>,
    alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    let cfg = PfdConfig::from_config(config)?;
    draw_pfd(data, &cfg, alerts, scene)?;
    Ok(())
}

fn draw_hsi_panel(
    data: &PanelData,
    config: &ConfigBlob<'_>,
    alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    // The HSI takes no configuration; the empty schema makes any keyed
    // blob a shell-side rejection before this runs, and a re-check here
    // keeps the property when a shell skips its gate.
    config.require_schema(HSI_DESCRIPTOR.config_schema)?;
    draw_hsi(data, alerts, scene)?;
    Ok(())
}

/// The primary flight display.
pub const PFD_DESCRIPTOR: PanelDescriptor = PanelDescriptor {
    id: "pfd",
    title: "PFD",
    required_layers: layer_bit(LayerId::Attitude)
        | layer_bit(LayerId::Tapes)
        | layer_bit(LayerId::Annunciation),
    required_groups: GroupSet::of(&[
        GroupId::Attitude,
        GroupId::Kinematics,
        GroupId::Air,
        GroupId::Selections,
        GroupId::Trust,
        GroupId::Altitude,
        GroupId::Dynamics,
    ]),
    design_frame: DesignFrame {
        width: PANEL_W,
        height: PANEL_H,
    },
    background: BackgroundCapability::Cedeable,
    config_schema: PFD_CONFIG_SCHEMA,
    group_regions: &[],
    extreme_states: &[],
    // Reference-rasterizer frame hash over the shared typical state —
    // pinned per panel here so a panel travels with its own regression
    // baseline; the raster crate asserts it (REN-03).
    raster_baseline: Some("43b49bde6bbf7372d704d54214d4a3d0b9cd3ad09e86862a8ffc20fd6ae05ef1"),
    draw: draw_pfd_panel,
};

/// The horizontal situation indicator.
pub const HSI_DESCRIPTOR: PanelDescriptor = PanelDescriptor {
    id: "hsi",
    title: "HSI",
    required_layers: layer_bit(LayerId::Attitude)
        | layer_bit(LayerId::Tapes)
        | layer_bit(LayerId::Guidance)
        | layer_bit(LayerId::Annunciation),
    required_groups: GroupSet::of(&[
        GroupId::Kinematics,
        GroupId::Nav,
        GroupId::Wind,
        GroupId::Selections,
        GroupId::Trust,
        GroupId::Heading,
        GroupId::Variation,
    ]),
    design_frame: DesignFrame {
        width: PANEL_W,
        height: PANEL_H,
    },
    background: BackgroundCapability::Opaque,
    config_schema: &[],
    group_regions: &[],
    extreme_states: &[],
    raster_baseline: Some("66653ce135e6f2163fa48d805a0ab1a8f3d0ac51d778f7b1eb2aa4ec05bfbb7c"),
    draw: draw_hsi_panel,
};

/// The panels this crate ships, in shell display order.
pub const BUILTIN_PANELS: &[PanelDescriptor] = &[PFD_DESCRIPTOR, HSI_DESCRIPTOR];

#[cfg(test)]
mod digest_tests;
#[cfg(test)]
mod tests;
