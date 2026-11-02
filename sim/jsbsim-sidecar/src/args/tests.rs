//! Command-line parsing.

#![allow(clippy::unwrap_used)]

use super::{Args, ArgsError};
use crate::aircraft::Aircraft;
use crate::engine::EngineKind;

fn argv(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn defaults_stand_an_x500_on_n51_for_px4_instance_0() {
    let args = Args::parse(&[]).unwrap();
    assert_eq!(args.port, 4560);
    assert_eq!(args.aircraft, Aircraft::X500);
    assert_eq!(args.site, pilotage_world::N51);
    assert_eq!(args.latitude_deg, pilotage_world::N51.latitude_deg);
    assert_eq!(args.realtime_factor, 1.0);
    assert_eq!(args.step_ms, 4);
    assert!(args.terrain.is_none());
}

#[test]
fn every_flag_parses() {
    let args = Args::parse(&argv(&[
        "--port",
        "4580",
        "--aircraft",
        "rascal",
        "--aircraft-dir",
        "/px4/models/Rascal",
        "--jsbsim-root",
        "/jsbsim",
        "--latitude-deg",
        "40.5",
        "--longitude-deg",
        "-74.7",
        "--heading-deg",
        "29.5",
        "--site-elevation-m",
        "57",
        "--terrain",
        "/t.mbtiles",
        "--epoch-unix-s",
        "1767225600",
        "--seed",
        "9",
        "--realtime-factor",
        "0",
        "--lockstep-timeout-s",
        "0.5",
        "--gimbal",
        "180,45,-135,720",
        "--truth-addr",
        "127.0.0.1:19580",
        "--step-ms",
        "1",
    ]))
    .unwrap();
    assert_eq!(args.port, 4580);
    assert_eq!(args.aircraft, Aircraft::Rascal);
    assert_eq!(args.site.elevation_m, 57.0);
    assert_eq!(args.epoch_unix_s, Some(1_767_225_600));
    assert_eq!(args.seed, 9);
    assert_eq!(args.lockstep_timeout.as_millis(), 500);
    assert_eq!(args.truth_addr.map(|a| a.port()), Some(19_580));
    assert_eq!(args.step_ms, 1);
    let gimbal = args.gimbal.unwrap();
    assert_eq!((gimbal.pitch.min_deg, gimbal.yaw.max_deg), (-135.0, 360.0));
}

#[test]
fn the_rust_engine_flies_unless_the_cpp_one_is_named() {
    assert_eq!(Args::parse(&[]).unwrap().engine, EngineKind::Rust);
    let cpp = Args::parse(&argv(&["--engine", "cpp"])).unwrap();
    assert_eq!(cpp.engine, EngineKind::Cpp);
    assert!(matches!(
        Args::parse(&argv(&["--engine", "fortran"])),
        Err(ArgsError::Invalid { .. })
    ));
}

#[test]
fn malformed_arguments_are_refused() {
    for bad in [
        argv(&["--aircraft", "cessna"]),
        argv(&["--port", "70000"]),
        argv(&["--heading-deg", "NaN"]),
        argv(&["--latitude-deg", "91"]),
        argv(&["--seed"]),
        argv(&["--fly"]),
        argv(&["--gimbal", "180,45,-135"]),
        argv(&["--step-ms", "3"]),
        argv(&["--gimbal", "180,-135,45,720"]),
    ] {
        assert!(Args::parse(&bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        Args::parse(&argv(&["--fly"])),
        Err(ArgsError::Unrecognized("--fly".to_owned()))
    );
}
