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
    Region,
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
    // Value-readout surfaces, keyed by the group whose data the number
    // comes from: the honest-status family proves these stay
    // numeral-free when that group is withheld. Scale ladders and the
    // attitude ball carry no per-group numerals and stay undeclared.
    group_regions: &[
        // IAS pointed readout value (the run anchors at x 40; the
        // scale ladder's runs anchor at x 70 and stay outside).
        (
            GroupId::Air,
            Region {
                x: 20.0,
                y: 162.0,
                width: 40.0,
                height: 36.0,
            },
        ),
        // Baro setting box.
        (
            GroupId::Air,
            Region {
                x: 390.0,
                y: 335.0,
                width: 90.0,
                height: 25.0,
            },
        ),
        // Groundspeed box.
        (
            GroupId::Kinematics,
            Region {
                x: 0.0,
                y: 335.0,
                width: 90.0,
                height: 25.0,
            },
        ),
        // Altitude pointed readout value (anchors at x 442; the scale
        // ladder anchors at x 408 and stays outside). The value is
        // kinematic altitude; the altitude group only qualifies its
        // datum.
        (
            GroupId::Kinematics,
            Region {
                x: 424.0,
                y: 162.0,
                width: 36.0,
                height: 36.0,
            },
        ),
        // Selected-altitude box.
        (
            GroupId::Selections,
            Region {
                x: 390.0,
                y: 0.0,
                width: 90.0,
                height: 24.0,
            },
        ),
        // VSI numeral strip (kinematic vertical speed), bounded to
        // exclude the selected-altitude box above and the baro box
        // below, whose numerals belong to other groups.
        (
            GroupId::Kinematics,
            Region {
                x: 440.0,
                y: 28.0,
                width: 26.0,
                height: 307.0,
            },
        ),
    ],
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
    group_regions: &[
        // Wind box.
        (
            GroupId::Wind,
            Region {
                x: 2.0,
                y: 2.0,
                width: 112.0,
                height: 48.0,
            },
        ),
        // Distance box.
        (
            GroupId::Nav,
            Region {
                x: 366.0,
                y: 2.0,
                width: 112.0,
                height: 48.0,
            },
        ),
        // Course box.
        (
            GroupId::Nav,
            Region {
                x: 2.0,
                y: 322.0,
                width: 112.0,
                height: 36.0,
            },
        ),
        // Heading-select box.
        (
            GroupId::Selections,
            Region {
                x: 366.0,
                y: 322.0,
                width: 112.0,
                height: 36.0,
            },
        ),
        // Digital heading readout at the panel top: the panel's
        // primary heading number must dash out with the sample gone.
        (
            GroupId::Heading,
            Region {
                x: 206.0,
                y: 2.0,
                width: 68.0,
                height: 26.0,
            },
        ),
    ],
    extreme_states: &[],
    raster_baseline: Some("66653ce135e6f2163fa48d805a0ab1a8f3d0ac51d778f7b1eb2aa4ec05bfbb7c"),
    draw: draw_hsi_panel,
};

fn draw_monitor_panel(
    data: &PanelData,
    config: &ConfigBlob<'_>,
    alerts: Option<&AlertOutput>,
    scene: &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError> {
    config.require_schema(MONITOR_DESCRIPTOR.config_schema)?;
    crate::monitor::draw_monitor(data, alerts, scene)?;
    Ok(())
}

/// The machine-monitoring text panel (AIR-IN-014) — the registry's
/// proof of modularity: it exists as this descriptor and a draw
/// function, with no shell change beyond composition.
pub const MONITOR_DESCRIPTOR: PanelDescriptor = PanelDescriptor {
    id: "monitor",
    title: "Monitor",
    required_layers: layer_bit(LayerId::Tapes) | layer_bit(LayerId::Annunciation),
    required_groups: GroupSet::of(&[GroupId::MonitorText, GroupId::Trust]),
    design_frame: DesignFrame {
        width: PANEL_W,
        height: PANEL_H,
    },
    background: BackgroundCapability::NotUsed,
    config_schema: &[],
    // The whole text area is the channel's region: with MONITOR_TEXT
    // withheld the panel shows dashes, never lines it was not given.
    group_regions: &[(
        GroupId::MonitorText,
        Region {
            x: 0.0,
            y: 60.0,
            width: 480.0,
            height: 300.0,
        },
    )],
    extreme_states: &[],
    raster_baseline: Some("6f554f502cd05f77526194a180ab93d5fbcdd26ba578f6216d281ff3125da8ec"),
    draw: draw_monitor_panel,
};

/// The panels this crate ships, in shell display order.
pub const BUILTIN_PANELS: &[PanelDescriptor] =
    &[PFD_DESCRIPTOR, HSI_DESCRIPTOR, MONITOR_DESCRIPTOR];

/// The pinned cross-shell scene digest over [`BUILTIN_PANELS`] and the
/// canonical corpus (ADR-0033). Every shell reports exactly this value
/// or it is not showing these instruments; it moves once per
/// deliberate contract change, re-pinned with a review note saying
/// why.
pub const BUILTIN_SCENE_DIGEST: &str =
    "57e6049a1905d720dec6756acf54034ced9855a015549189c0016626933bc368";

#[cfg(test)]
mod digest_tests;
#[cfg(test)]
mod tests;
