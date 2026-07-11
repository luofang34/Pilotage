#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec::Vec;

use pilotage_instrument_scene::{Cmd, SceneCmds, SceneWriter};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, FreshnessPolicy, Kinematics, PanelData, Quat, Stamped,
    resolve,
};

use super::{PfdConfig, VSpeeds, draw_pfd};

fn flying() -> PanelData {
    let state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0, 0.0, 0.02],
            }),
            age_ms: Some(20.0),
        },
        kinematics: Stamped {
            data: Some(Kinematics {
                pos_ned_m: [0.0, 0.0, -300.0],
                vel_ned_mps: [20.0, 0.0, -1.0],
            }),
            age_ms: Some(20.0),
        },
        air: Stamped {
            data: Some(AirData {
                ias_mps: Some(40.0),
                baro_setting_hpa: Some(1013.0),
            }),
            age_ms: Some(20.0),
        },
        ..AircraftState::default()
    };
    resolve(&state, &FreshnessPolicy::default())
}

fn render(data: &PanelData, cfg: &PfdConfig) -> Vec<u8> {
    let mut buf = std::vec![0u8; 32 * 1024];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    draw_pfd(data, cfg, &mut w).expect("panel fits buffer");
    let len = w.finish();
    buf.truncate(len);
    buf
}

fn texts(scene: &[u8]) -> Vec<String> {
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter_map(|c| match c {
            Cmd::Text { text, .. } => Some(String::from(text)),
            _ => None,
        })
        .collect()
}

fn layer_texts(scene: &[u8], wanted: LayerId) -> Vec<(String, [f32; 3])> {
    let mut inside = false;
    let mut found = Vec::new();
    for command in SceneCmds::new(scene).expect("valid scene") {
        match command.expect("valid command") {
            Cmd::BeginLayer { layer } => inside = layer == wanted,
            Cmd::EndLayer { layer } if layer == wanted => inside = false,
            Cmd::Text {
                x, y, size, text, ..
            } if inside => found.push((String::from(text), [x, y, size])),
            _ => {}
        }
    }
    found
}

fn save_restore_balance(scene: &[u8]) -> i32 {
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .fold(0i32, |acc, c| match c {
            Cmd::Save => acc + 1,
            Cmd::Restore => acc - 1,
            _ => acc,
        })
}

#[test]
fn valid_state_renders_decodable_balanced_scene() {
    let scene = render(&flying(), &PfdConfig::default());
    assert_eq!(save_restore_balance(&scene), 0);
    let labels = texts(&scene);
    // IAS readout: 40 m/s ≈ 078 kt.
    assert!(labels.iter().any(|t| t == "078"), "IAS readout: {labels:?}");
    // Altitude readout: 300 m ≈ 980 ft (rounded to 10).
    assert!(labels.iter().any(|t| t == "980"), "ALT readout: {labels:?}");
    // No failure dashes anywhere.
    assert!(!labels.iter().any(|t| t == "---"));
}

#[test]
fn missing_attitude_renders_red_x_not_horizon() {
    let mut data = flying();
    data.roll_rad.status = pilotage_instrument_state::SignalStatus::Missing;
    let scene = render(&data, &PfdConfig::default());
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "ATT"), "ATT flag: {labels:?}");
    assert!(
        layer_texts(&scene, LayerId::Annunciation)
            .contains(&(String::from("ATT"), [240.0, 170.0, 20.0])),
        "ATT failure must be an annunciation"
    );
    assert_eq!(save_restore_balance(&scene), 0);
}

#[test]
fn missing_airspeed_shows_dashes() {
    let mut data = flying();
    data.ias_kt = pilotage_instrument_state::Sig::missing();
    let scene = render(&data, &PfdConfig::default());
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "---"), "dashes: {labels:?}");
    assert!(labels.iter().any(|t| t == "IAS"), "IAS flag: {labels:?}");
}

#[test]
fn v_speed_bands_add_rects_not_errors() {
    let cfg = PfdConfig {
        v_speeds: Some(VSpeeds {
            vs0_kt: 40.0,
            vs_kt: 48.0,
            vfe_kt: 85.0,
            vno_kt: 129.0,
            vne_kt: 163.0,
        }),
        ..PfdConfig::default()
    };
    let bare = render(&flying(), &PfdConfig::default());
    let banded = render(&flying(), &cfg);
    assert!(banded.len() > bare.len());
}

#[test]
fn empty_state_still_renders_a_scene() {
    let data = resolve(&AircraftState::default(), &FreshnessPolicy::default());
    let scene = render(&data, &PfdConfig::default());
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "ATT"));
    assert_eq!(save_restore_balance(&scene), 0);
}

// ---- REN-01 layer contract ---------------------------------------------------

use pilotage_instrument_scene::{LayerId, validate_layers};
use pilotage_instrument_state::SignalStatus;

use super::BackgroundMode;

const PFD_CRITICAL: [LayerId; 3] = [LayerId::Attitude, LayerId::Tapes, LayerId::Annunciation];

#[test]
fn scenes_are_layered_for_every_attitude_status() {
    for status in [
        SignalStatus::Valid,
        SignalStatus::Degraded,
        SignalStatus::Stale,
        SignalStatus::Missing,
        SignalStatus::Failed,
    ] {
        let mut data = flying();
        data.roll_rad.status = status;
        data.pitch_rad.status = status;
        let scene = render(&data, &PfdConfig::default());
        let report = validate_layers(&scene).expect("layered scene validates");
        assert!(report.contains(LayerId::Background), "{status:?}");
        for layer in PFD_CRITICAL {
            assert!(report.contains(layer), "{status:?} missing {layer:?}");
        }
    }
}

#[test]
fn critical_overlay_is_byte_identical_without_background() {
    for status in [
        SignalStatus::Valid,
        SignalStatus::Degraded,
        SignalStatus::Stale,
        SignalStatus::Missing,
        SignalStatus::Failed,
    ] {
        let mut data = flying();
        data.roll_rad.status = status;
        data.pitch_rad.status = status;
        let with_horizon = render(&data, &PfdConfig::default());
        let without = render(
            &data,
            &PfdConfig {
                background: BackgroundMode::None,
                ..PfdConfig::default()
            },
        );
        let horizon_report = validate_layers(&with_horizon).expect("validates");
        let bare_report = validate_layers(&without).expect("validates");
        assert!(!bare_report.contains(LayerId::Background));
        for layer in PFD_CRITICAL {
            let (hs, he) = horizon_report.ranges[layer.to_u8() as usize].expect("range");
            let (bs, be) = bare_report.ranges[layer.to_u8() as usize].expect("range");
            assert_eq!(
                &with_horizon[hs..he],
                &without[bs..be],
                "{status:?} layer {layer:?} content changed with the background"
            );
        }
        if status.shows_value() {
            let attitude_text = layer_texts(&without, LayerId::Attitude);
            assert!(
                attitude_text.iter().any(|(text, _)| text == "10"),
                "{status:?} background-free PFD lost its pitch ladder"
            );
            assert!(
                !layer_texts(&with_horizon, LayerId::Background)
                    .iter()
                    .any(|(text, _)| text == "10"),
                "{status:?} pitch ladder must not belong to Background"
            );
        }
    }
}

#[test]
fn air_data_failure_cues_are_annunciations() {
    let mut data = flying();
    data.ias_kt =
        pilotage_instrument_state::Sig::with_status(data.ias_kt.value, SignalStatus::Failed);
    data.alt_ft =
        pilotage_instrument_state::Sig::with_status(data.alt_ft.value, SignalStatus::Failed);
    let scene = render(&data, &PfdConfig::default());
    let annunciations = layer_texts(&scene, LayerId::Annunciation);
    let tapes = layer_texts(&scene, LayerId::Tapes);
    for expected in [("IAS", [45.0, 160.0, 20.0]), ("ALT", [435.0, 160.0, 20.0])] {
        assert!(
            annunciations
                .iter()
                .any(|(text, geometry)| text == expected.0 && *geometry == expected.1),
            "missing annunciation {expected:?}: {annunciations:?}"
        );
        assert!(
            !tapes
                .iter()
                .any(|(text, geometry)| text == expected.0 && *geometry == expected.1),
            "failure cue leaked into tapes: {tapes:?}"
        );
    }
}
