#![allow(clippy::expect_used, clippy::panic)]

use super::{Args, ArgsError};

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn defaults_put_the_quad_at_the_demo_origin_on_betaflight_ports() {
    let args = Args::parse(&[]).expect("empty parses");
    assert_eq!(args.motor_listen.port(), 9_002);
    assert_eq!(args.state_target.port(), 9_003);
    assert_eq!(args.origin.lat_deg, 47.397_742);
    assert_eq!(args.origin.lon_deg, 8.545_594);
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
