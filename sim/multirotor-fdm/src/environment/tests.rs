#![allow(clippy::expect_used, clippy::panic)]

use world_atmosphere::{Altitude, Isa};

use super::{Environment, EnvironmentError, Origin};
use crate::rotation::Vec3;

const ORIGIN: Origin = Origin {
    lat_deg: 47.397_742,
    lon_deg: 8.545_594,
    alt_m: 488.0,
};

fn environment() -> Environment {
    Environment::new(ORIGIN).expect("well-formed origin")
}

#[test]
fn gravity_is_the_world_model_normal_gravity_at_the_origin() {
    let expected = world_core::normal_gravity(world_core::GeodeticPosition::wgs84(
        ORIGIN.lat_deg,
        ORIGIN.lon_deg,
        ORIGIN.alt_m,
    ))
    .expect("gravity");
    assert_eq!(environment().gravity_mps2(), expected);
    assert!((9.80..9.81).contains(&expected), "{expected}");
}

#[test]
fn local_and_geodetic_round_trip_through_the_world_frame() {
    let environment = environment();
    for (n, e, d) in [(0.0, 0.0, 0.0), (35.0, -20.0, -12.5), (-400.0, 250.0, 3.0)] {
        let position = environment.geodetic(Vec3::new(n, e, d));
        let back = environment.frame().to_ned(position).expect("same datum");
        assert!((back.north_m - n).abs() < 1e-6, "{back:?}");
        assert!((back.east_m - e).abs() < 1e-6, "{back:?}");
        assert!((back.down_m - d).abs() < 1e-6, "{back:?}");
    }
    let origin = environment.geodetic(Vec3::ZERO);
    assert!((origin.latitude_deg - ORIGIN.lat_deg).abs() < 1e-9);
    assert!((origin.ellipsoidal_height_m - ORIGIN.alt_m).abs() < 1e-6);
}

#[test]
fn pressure_is_the_standard_atmosphere_at_the_altitude() {
    let environment = environment();
    for down in [0.0, -150.0, 20.0] {
        let position = environment.geodetic(Vec3::new(10.0, 5.0, down));
        let expected = Isa
            .at(Altitude::GeometricMsl(position.ellipsoidal_height_m))
            .expect("in the troposphere")
            .pressure_pa;
        assert_eq!(environment.pressure_pa(position).expect("air"), expected);
    }
    let space = environment.geodetic(Vec3::new(0.0, 0.0, -200_000.0));
    assert!(matches!(
        environment.pressure_pa(space),
        Err(EnvironmentError::Atmosphere { .. })
    ));
}

#[test]
fn a_malformed_origin_is_refused() {
    let origin = Origin {
        lat_deg: 91.0,
        ..ORIGIN
    };
    assert!(matches!(
        Environment::new(origin),
        Err(EnvironmentError::Origin { .. })
    ));
}
