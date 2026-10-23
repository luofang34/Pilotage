use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_sim_multicopter::{ESRI_WORLD_IMAGERY, TileSourceConfig};

use super::{SimMulticopterHostError, config_from};

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn the_launch_point_is_required() {
    assert!(matches!(
        config_from(lookup(&[])),
        Err(SimMulticopterHostError::MissingLaunch)
    ));
}

#[test]
fn defaults_use_the_esri_imagery_and_the_shared_cache() {
    let config = config_from(lookup(&[("PILOTAGE_SIM_LAUNCH", "40.3317,-74.6745,40")]))
        .expect("the defaults parse");
    assert_eq!(config.launch.lat_deg, 40.3317);
    assert_eq!(config.launch.alt_m, 40.0);
    assert!(config.display_name().starts_with("SIM"));
    assert_eq!(config.camera.fps, 5);
    assert_eq!((config.camera.width, config.camera.height), (640, 360));
    assert_eq!(
        config.camera.tiles,
        TileSourceConfig::Http {
            template: ESRI_WORLD_IMAGERY.to_owned(),
            cache_dir: Some(std::env::temp_dir().join("pilotage-sim-tiles")),
        }
    );
}

#[test]
fn every_variable_is_read() {
    let config = config_from(lookup(&[
        ("PILOTAGE_SIM_LAUNCH", "40.33, -74.67, 35.5"),
        ("PILOTAGE_SIM_NAME", "alpha-2"),
        ("PILOTAGE_SIM_TILES_URL", "procedural"),
        ("PILOTAGE_SIM_CAMERA_FPS", "2"),
        ("PILOTAGE_SIM_CAMERA_FAULT", "dark@10-20s"),
        ("PILOTAGE_SIM_BATTERY_WH", "40"),
        ("PILOTAGE_SIM_SEED", "17"),
        ("PILOTAGE_SIM_GIMBAL_PITCH", "-45"),
        ("PILOTAGE_SIM_FRAME_DUMP", "/tmp/frames"),
    ]))
    .expect("parses");
    assert_eq!(config.display_name(), "SIM alpha-2");
    assert_eq!(config.camera.tiles, TileSourceConfig::Procedural);
    assert_eq!(config.camera.fps, 2);
    assert_eq!(config.camera.faults.len(), 1);
    assert_eq!(config.battery.capacity_wh, 40.0);
    assert_eq!(config.seed, 17);
    assert_eq!(config.initial_gimbal_pitch_deg, -45.0);
    assert_eq!(config.camera.frame_dump, Some(PathBuf::from("/tmp/frames")));
}

#[test]
fn a_custom_template_keeps_the_cache_directory() {
    let config = config_from(lookup(&[
        ("PILOTAGE_SIM_LAUNCH", "40.33,-74.67,35"),
        (
            "PILOTAGE_SIM_TILES_URL",
            "http://localhost:8080/{z}/{x}/{y}.png",
        ),
        ("PILOTAGE_SIM_TILE_CACHE", "/var/tmp/tiles"),
    ]))
    .expect("parses");
    assert_eq!(
        config.camera.tiles,
        TileSourceConfig::Http {
            template: "http://localhost:8080/{z}/{x}/{y}.png".to_owned(),
            cache_dir: Some(PathBuf::from("/var/tmp/tiles")),
        }
    );
}

#[test]
fn malformed_values_are_refused() {
    let launch = ("PILOTAGE_SIM_LAUNCH", "40.33,-74.67,35");
    for (name, value) in [
        ("PILOTAGE_SIM_LAUNCH", "40.33,-74.67"),
        ("PILOTAGE_SIM_LAUNCH", "95,-74.67,10"),
        ("PILOTAGE_SIM_NAME", ""),
        ("PILOTAGE_SIM_TILES_URL", "ftp://tiles/{z}/{x}/{y}"),
        ("PILOTAGE_SIM_TILES_URL", "https://tiles/{z}/{x}"),
        ("PILOTAGE_SIM_CAMERA_FPS", "0"),
        ("PILOTAGE_SIM_CAMERA_FPS", "2.5"),
        ("PILOTAGE_SIM_CAMERA_FAULT", "fog@1-2s"),
        ("PILOTAGE_SIM_BATTERY_WH", "-1"),
        ("PILOTAGE_SIM_SEED", "seven"),
        ("PILOTAGE_SIM_GIMBAL_PITCH", "-120"),
    ] {
        let pairs = if name == launch.0 {
            vec![(name, value)]
        } else {
            vec![launch, (name, value)]
        };
        assert!(
            config_from(lookup(&pairs)).is_err(),
            "{name}={value} parsed"
        );
    }
}
