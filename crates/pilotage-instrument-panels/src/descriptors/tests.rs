#![allow(clippy::expect_used, clippy::panic)]

use std::vec;
use std::vec::Vec;

use pilotage_instrument_registry::{ConfigBlob, EMPTY_CONFIG, Registry, keys};
use pilotage_instrument_scene::SceneWriter;
use pilotage_instrument_state::{
    AircraftState, Attitude, FreshnessPolicy, PanelData, Quat, Stamped, resolve,
};

use super::{BUILTIN_PANELS, HSI_DESCRIPTOR, PFD_DESCRIPTOR};
use crate::{BackgroundMode, PfdConfig, draw_hsi, draw_pfd};

fn resolved() -> PanelData {
    let state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rates_rps: [0.0, 0.0, 0.0],
            }),
            age_ms: Some(50.0),
        },
        ..AircraftState::default()
    };
    resolve(&state, &FreshnessPolicy::default())
}

fn scene_via_descriptor(
    descriptor: &pilotage_instrument_registry::PanelDescriptor,
    config: &ConfigBlob<'_>,
) -> Vec<u8> {
    let data = resolved();
    let mut buf = vec![0u8; 64 * 1024];
    let mut writer = SceneWriter::new(&mut buf).expect("fits");
    (descriptor.draw)(&data, config, None, &mut writer).expect("draws");
    let used = writer.finish();
    buf[..used].to_vec()
}

#[test]
fn the_builtin_composition_validates() {
    Registry::new(BUILTIN_PANELS).expect("shipped panels must compose");
}

#[test]
fn descriptor_draws_match_the_direct_entry_points() {
    let data = resolved();
    let mut direct_buf = vec![0u8; 64 * 1024];
    let mut writer = SceneWriter::new(&mut direct_buf).expect("fits");
    draw_pfd(&data, &PfdConfig::default(), None, &mut writer).expect("draws");
    let used = writer.finish();
    assert_eq!(
        scene_via_descriptor(&PFD_DESCRIPTOR, &EMPTY_CONFIG),
        direct_buf[..used].to_vec(),
        "descriptor PFD must be the same panel as draw_pfd"
    );

    let mut hsi_buf = vec![0u8; 64 * 1024];
    let mut writer = SceneWriter::new(&mut hsi_buf).expect("fits");
    draw_hsi(&data, None, &mut writer).expect("draws");
    let used = writer.finish();
    assert_eq!(
        scene_via_descriptor(&HSI_DESCRIPTOR, &EMPTY_CONFIG),
        hsi_buf[..used].to_vec(),
        "descriptor HSI must be the same panel as draw_hsi"
    );
}

#[test]
fn svs_background_cedes_exactly_like_none() {
    // Accept-and-cede (ADR-0033): until the SVS renderer exists, an
    // SVS-configured PFD emits byte-identical scenes to a None
    // background — nothing above the Background band may depend on it.
    let data = resolved();
    let mut scenes = Vec::new();
    for background in [
        BackgroundMode::None,
        BackgroundMode::Svs {
            viewport: crate::SvsViewport {
                x: 0.0,
                y: 0.0,
                width: 480.0,
                height: 360.0,
            },
            quality: 2,
        },
    ] {
        let cfg = PfdConfig {
            background,
            v_speeds: None,
        };
        let mut buf = vec![0u8; 64 * 1024];
        let mut writer = SceneWriter::new(&mut buf).expect("fits");
        draw_pfd(&data, &cfg, None, &mut writer).expect("draws");
        let used = writer.finish();
        scenes.push(buf[..used].to_vec());
    }
    assert_eq!(scenes[0], scenes[1]);
}

#[test]
fn an_svs_config_blob_decodes_and_still_cedes() {
    // BACKGROUND_MODE=2 with a viewport and quality: the decoded config
    // must carry the request, and the drawn scene must equal None's.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(2);
    bytes.extend_from_slice(&keys::SVS_VIEWPORT.0.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    for v in [0.0f32, 0.0, 480.0, 240.0] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&keys::SVS_QUALITY.0.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(1);
    let blob = ConfigBlob::parse(&bytes).expect("well-formed");
    let cfg = PfdConfig::from_config(&blob).expect("decodes");
    assert!(matches!(
        cfg.background,
        BackgroundMode::Svs { quality: 1, .. }
    ));

    let via_svs = scene_via_descriptor(&PFD_DESCRIPTOR, &blob);
    let none_bytes = {
        let mut b = Vec::new();
        b.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.push(1);
        b
    };
    let none_blob = ConfigBlob::parse(&none_bytes).expect("well-formed");
    assert_eq!(via_svs, scene_via_descriptor(&PFD_DESCRIPTOR, &none_blob));
}

#[test]
fn svs_keys_are_validated_and_refused_when_inert() {
    use pilotage_instrument_registry::ConfigError;
    // A malformed viewport payload is refused even when the selected
    // background never consumes it.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&keys::SVS_VIEWPORT.0.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&[1, 2, 3]);
    let blob = ConfigBlob::parse(&bytes).expect("well-formed framing");
    assert_eq!(
        PfdConfig::from_config(&blob),
        Err(ConfigError::BadValue {
            key: keys::SVS_VIEWPORT.0,
            len: 3,
        })
    );
    // A well-formed viewport under a non-SVS background is inert, and
    // inert is refused rather than silently dropped.
    let mut inert = Vec::new();
    inert.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
    inert.extend_from_slice(&1u16.to_le_bytes());
    inert.push(0);
    inert.extend_from_slice(&keys::SVS_VIEWPORT.0.to_le_bytes());
    inert.extend_from_slice(&16u16.to_le_bytes());
    for v in [0.0f32, 0.0, 480.0, 360.0] {
        inert.extend_from_slice(&v.to_le_bytes());
    }
    let blob = ConfigBlob::parse(&inert).expect("well-formed framing");
    assert_eq!(
        PfdConfig::from_config(&blob),
        Err(ConfigError::InertKey {
            key: keys::SVS_VIEWPORT.0,
        })
    );
    // A viewport outside the design frame is refused.
    let mut outside = Vec::new();
    outside.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
    outside.extend_from_slice(&1u16.to_le_bytes());
    outside.push(2);
    outside.extend_from_slice(&keys::SVS_VIEWPORT.0.to_le_bytes());
    outside.extend_from_slice(&16u16.to_le_bytes());
    for v in [-10.0f32, 0.0, 480.0, 360.0] {
        outside.extend_from_slice(&v.to_le_bytes());
    }
    let blob = ConfigBlob::parse(&outside).expect("well-formed framing");
    assert_eq!(
        PfdConfig::from_config(&blob),
        Err(ConfigError::BadValue {
            key: keys::SVS_VIEWPORT.0,
            len: 16,
        })
    );
}

#[test]
fn a_collapsed_v_speed_ladder_is_refused() {
    use pilotage_instrument_registry::ConfigError;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&keys::V_SPEEDS.0.to_le_bytes());
    bytes.extend_from_slice(&20u16.to_le_bytes());
    for v in [0.0f32, 0.0, 0.0, 0.0, 0.5] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let blob = ConfigBlob::parse(&bytes).expect("well-formed framing");
    assert_eq!(
        PfdConfig::from_config(&blob),
        Err(ConfigError::BadValue {
            key: keys::V_SPEEDS.0,
            len: 20,
        })
    );
}

#[test]
fn the_hsi_rejects_any_configuration_key() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&keys::BACKGROUND_MODE.0.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(0);
    let blob = ConfigBlob::parse(&bytes).expect("well-formed");
    let data = resolved();
    let mut buf = vec![0u8; 64 * 1024];
    let mut writer = SceneWriter::new(&mut buf).expect("fits");
    assert!((HSI_DESCRIPTOR.draw)(&data, &blob, None, &mut writer).is_err());
}
