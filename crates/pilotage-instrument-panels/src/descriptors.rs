//! Built-in panel descriptors: the registry entries every shell
//! composes (ADR-0029, ADR-0033).
//!
//! Each descriptor owns its panel's full contract — identity, masks,
//! required groups, honest-status regions, its own extreme states, and
//! the pinned raster baseline — so a shell consumes composition data
//! and never holds a panel list, index, or mask of its own.

use pilotage_alerts::AlertOutput;
use pilotage_instrument_registry::{
    BackgroundCapability, ConfigBlob, DesignFrame, GroupSet, PanelDescriptor, PanelDrawError,
    Region,
};
use pilotage_instrument_scene::{LayerId, SceneWriter};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, DynSample, GroupId, HeadingReference, HeadingSample,
    Kinematics, MonitorText, NavData, NavFromTo, NavSource, PanelData, Quat, Stamped, TextLine,
    TurnBasis, TurnSample,
};

use pilotage_instrument_registry::{ExtremeState, states};

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
    // comes from. Honest status is proven by provenance claims on the
    // runs themselves (the harness's withholding matrix tests every
    // claim, wherever the ink lands), so these regions are the
    // descriptor's statement of readout ownership for a shell — not
    // the numeral police.
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
        // The selected-altitude box carries no region: it shares the
        // altitude tape strip with kinematics ladder ink whose y moves
        // with altitude, so no region geometry separates the two. Its
        // honest-status coverage is the provenance claim on the
        // selected-altitude run itself — a fabricated selection is
        // refused wherever it is drawn.
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
    extreme_states: &[
        ExtremeState {
            id: "unusual-inverted",
            build: pfd_unusual_inverted,
        },
        ExtremeState {
            id: "readout-extremes",
            build: pfd_readout_extremes,
        },
    ],
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
    extreme_states: &[
        ExtremeState {
            id: "reciprocal-course",
            build: hsi_reciprocal_course,
        },
        ExtremeState {
            id: "track-up",
            build: hsi_track_up,
        },
    ],
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
    required_groups: GroupSet::of(&[GroupId::MonitorText]),
    design_frame: DesignFrame {
        width: PANEL_W,
        height: PANEL_H,
    },
    // The panel owns its band with an opaque ground: text needs it, and
    // declaring anything weaker would hand a compositor a black
    // rectangle it was told is not painted.
    background: BackgroundCapability::Opaque,
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
    extreme_states: &[ExtremeState {
        id: "full-channel",
        build: monitor_full_channel,
    }],
    raster_baseline: Some("40f44383f3ad46a0bbd65f04afc1d80fb9d94c11acff8dc66edbfcf7b8fa4c01"),
    draw: draw_monitor_panel,
};

/// Inverted, nose-low, rolling hard: the unusual-attitude tier, the
/// recovery chevrons, and the pitch ladder far from level — the PFD's
/// own hardest drawing, unreachable from the gentle shared corpus.
fn pfd_unusual_inverted() -> AircraftState {
    let mut state = states::typical();
    state.attitude = Stamped {
        data: Some(Attitude {
            quat: Quat::from_euler(2.8, -0.9, 4.0),
            rates_rps: [1.5, -0.8, 0.9],
        }),
        age_ms: Some(40.0),
    };
    state.dynamics = Stamped {
        data: Some(DynSample {
            turn: Some(TurnSample {
                rate_rps: -0.6,
                basis: TurnBasis::HeadingRate,
            }),
            lateral_mps2: 3.5.into(),
        }),
        age_ms: Some(40.0),
    };
    state
}

/// Wide and negative readout values — the DISP-02 fit cases ("10300",
/// "-1030"-class) — plus the heading on the 360/0 wrap.
fn pfd_readout_extremes() -> AircraftState {
    let mut state = states::typical();
    state.air = Stamped {
        data: Some(AirData {
            ias_mps: Some(199.0),
            baro_setting_hpa: Some(1049.7),
        }),
        age_ms: Some(40.0),
    };
    state.kinematics = Stamped {
        data: Some(Kinematics {
            pos_ned_m: [0.0, 0.0, 320.0],
            vel_ned_mps: [-90.0, -2.0, 18.0],
        }),
        age_ms: Some(40.0),
    };
    state.heading = Stamped {
        data: Some(HeadingSample {
            heading_rad: 6.2828,
            reference: HeadingReference::SimLocalTrue,
        }),
        age_ms: Some(40.0),
    };
    state
}

/// Course exactly reciprocal to the flown track, full-scale deviation,
/// a zero-distance waypoint, and a heading on the 360/0 wrap.
fn hsi_reciprocal_course() -> AircraftState {
    let mut state = states::typical();
    state.nav = Stamped {
        data: Some(NavData {
            source: NavSource::Gps,
            // Exactly pi from the wrapped heading this fixture sets:
            // the reciprocal the state id names, not an inherited one.
            course_rad: 3.1412,
            cdi_dots: -2.5,
            fromto: NavFromTo::From,
            vdev_dots: Some(2.5),
            dist_nm: Some(0.0),
            course_reference: HeadingReference::SimLocalTrue,
            ..NavData::default()
        }),
        age_ms: Some(40.0),
    };
    state.heading = Stamped {
        data: Some(HeadingSample {
            heading_rad: 6.2828,
            reference: HeadingReference::SimLocalTrue,
        }),
        age_ms: Some(40.0),
    };
    state
}

/// The data-gateway profile (#260): a certified GPS navigator bridged
/// over its serial protocol publishes position, track, and guidance —
/// and no magnetic heading at all. The rose must present track-up,
/// annunciated TRK, instead of going structurally inert.
fn hsi_track_up() -> AircraftState {
    let mut state = states::typical();
    state.heading = Stamped {
        data: None,
        age_ms: None,
    };
    state
}

/// Eight maximum-length lines: the channel's full frame budget against
/// the glyph vocabulary, with digits in every row for the honest-status
/// family to police.
fn monitor_full_channel() -> AircraftState {
    let mut state = states::typical();
    let mut lines = [TextLine::EMPTY; MonitorText::MAX_LINES];
    for (row, slot) in lines.iter_mut().enumerate() {
        let text = match row {
            0 => "0123456789 ABCDEFGHIJKLMNOPQRS-.",
            1 => "ENG 1 N1 101.5 EGT 899 FF 1204.7",
            2 => "ENG 2 N1 100.9 EGT 901 FF 1198.2",
            3 => "FUEL L 1250.5 R 1248.0 CTR 890.4",
            4 => "HYD A 2987 B 3011 ELEC 28.4 27.9",
            5 => "GEAR DOWN-LOCKED FLAPS 25 TRIM 4",
            6 => "CABIN ALT 6500 RATE -300 DIFF 7.",
            7 => "WXYZ-0123456789.0123456789-WXYZ.",
            _ => "",
        };
        *slot = TextLine::new(text).unwrap_or(TextLine::EMPTY);
    }
    state.monitor_text = Stamped {
        data: Some(MonitorText::new(9, &lines).unwrap_or_default()),
        age_ms: Some(120.0),
    };
    state
}

/// The panels this crate ships, in shell display order.
pub const BUILTIN_PANELS: &[PanelDescriptor] =
    &[PFD_DESCRIPTOR, HSI_DESCRIPTOR, MONITOR_DESCRIPTOR];

/// The pinned scene digest over [`BUILTIN_PANELS`] and the canonical
/// corpus (ADR-0033): the composition contract every build target must
/// reproduce — the host (bench and unit pin) and the wasm build (the
/// script pins the exported value against its own literal). A shell's
/// LIVE rendering shares identity with this corpus structurally, by
/// drawing through the same descriptors, rather than by digest. The
/// value moves once per deliberate contract change, re-pinned with a
/// review note saying why.
pub const BUILTIN_SCENE_DIGEST: &str =
    "a809f1768d03f533b134e15ddf1a49779565f03fc829164fb6c94b357bdb1abc";

#[cfg(test)]
mod digest_tests;
#[cfg(test)]
mod tests;
