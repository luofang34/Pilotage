#![allow(clippy::expect_used, clippy::panic)]

use super::{Args, ArgsError};

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn defaults_put_the_quad_at_n51_on_betaflight_ports() {
    let args = Args::parse(&[]).expect("empty parses");
    assert_eq!(args.motor_listen.port(), 9_002);
    assert_eq!(args.state_target.port(), 9_003);
    assert_eq!(args.origin.lat_deg, 40.577_186_1);
    assert_eq!(args.origin.lon_deg, -74.738_191_8);
    assert_eq!(args.origin.alt_m, 56.3);
}

#[test]
fn spawn_offsets_and_rate_parse() {
    let args = Args::parse(&argv(&[
        "--north",
        "-20",
        "--east",
        "30.5",
        "--heading-deg",
        "90",
        "--rate-hz",
        "500",
    ]))
    .expect("parses");
    assert_eq!((args.north_m, args.east_m), (-20.0, 30.5));
    assert!((args.heading_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!(args.rate_hz, 500.0);
}

#[test]
fn non_finite_values_and_unknown_flags_are_refused() {
    assert!(matches!(
        Args::parse(&argv(&["--east", "NaN"])),
        Err(ArgsError::Invalid { .. })
    ));
    assert!(matches!(
        Args::parse(&argv(&["--rate-hz", "50"])),
        Err(ArgsError::Invalid { .. })
    ));
    assert_eq!(
        Args::parse(&argv(&["--north"])),
        Err(ArgsError::MissingValue {
            flag: "--north".to_owned()
        })
    );
    assert_eq!(
        Args::parse(&argv(&["--port"])),
        Err(ArgsError::Unrecognized("--port".to_owned()))
    );
}

#[test]
fn the_betaflight_flags_name_the_child_to_supervise() {
    let args = Args::parse(&argv(&[
        "--betaflight",
        "/bf/betaflight_SITL.elf",
        "--betaflight-dir",
        "/tmp/bf",
        "--betaflight-config",
        "sitl.cli",
    ]))
    .expect("parses");
    let sitl = args.betaflight.expect("a child to supervise");
    assert_eq!(
        sitl.executable,
        std::path::PathBuf::from("/bf/betaflight_SITL.elf")
    );
    assert_eq!(sitl.work_dir, std::path::PathBuf::from("/tmp/bf"));
    assert_eq!(sitl.config, Some(std::path::PathBuf::from("sitl.cli")));
    assert!(Args::parse(&[]).expect("parses").betaflight.is_none());
    assert!(Args::parse(&argv(&["--betaflight-config", "sitl.cli"])).is_err());
}
