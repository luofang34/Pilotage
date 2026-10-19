//! Site tests: N51 agrees with its published runway, `--site` parses
//! names and stated sites, and the layout keeps every vehicle clear of
//! the runway and of one another.

#![allow(clippy::expect_used, clippy::panic)]

use super::{DEFAULT_SITE, Ground, N51, NAMED_SITES, Site, ZURICH};
use crate::backend::SpawnPoint;
use crate::error::XtaskError;

/// The far (north-east) end of the N51 asphalt, from the same
/// OpenStreetMap way as the threshold.
const N51_RUNWAY_22_END: (f64, f64) = (40.584_335_2, -74.732_882_9);

/// North and east metres from `from` to `to`, on WGS84 at their mean
/// latitude.
fn north_east_m(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    let (a, f) = (6_378_137.0_f64, 1.0 / 298.257_223_563);
    let e2 = f * (2.0 - f);
    let phi = ((from.0 + to.0) / 2.0).to_radians();
    let w = (1.0 - e2 * phi.sin().powi(2)).sqrt();
    let meridian = a * (1.0 - e2) / w.powi(3);
    let normal = a / w;
    (
        (to.0 - from.0).to_radians() * meridian,
        (to.1 - from.1).to_radians() * normal * phi.cos(),
    )
}

/// Where `point` is from the site's threshold: along the runway and to
/// its left, m.
fn along_left_m(site: &Site, point: &SpawnPoint) -> (f64, f64) {
    let (north, east) = north_east_m(
        (site.latitude_deg, site.longitude_deg),
        (point.latitude_deg, point.longitude_deg),
    );
    let heading = site.heading_deg.to_radians();
    (
        north * heading.cos() + east * heading.sin(),
        north * heading.sin() - east * heading.cos(),
    )
}

#[test]
fn the_default_site_is_n51_runway_04() {
    assert_eq!(DEFAULT_SITE, N51);
    let (north, east) = north_east_m((N51.latitude_deg, N51.longitude_deg), N51_RUNWAY_22_END);
    let course = east.atan2(north).to_degrees();
    assert!(
        (course - N51.heading_deg).abs() < 0.1,
        "runway 04 runs {course:.2}°"
    );
    assert!((course - 30.0).abs() < 1.0, "runway 04 is about 030°");
    // FAA: field elevation 189 ft, runway 04 threshold 180 ft MSL.
    assert!((50.0..=58.0).contains(&N51.elevation_m));
}

#[test]
fn named_and_stated_sites_parse() {
    for site in NAMED_SITES {
        assert_eq!(Site::parse(site.name).expect("named"), site);
    }
    assert_eq!(Site::parse("zurich").expect("zurich"), ZURICH);
    let stated = Site::parse("46.5, 7.25,1200,270").expect("stated");
    assert_eq!(
        (
            stated.name,
            stated.latitude_deg,
            stated.longitude_deg,
            stated.elevation_m,
            stated.heading_deg
        ),
        ("custom", 46.5, 7.25, 1_200.0, 270.0)
    );
}

#[test]
fn a_malformed_site_is_refused_naming_the_choices() {
    for spec in [
        "n52",
        "46.5,7.25,1200",
        "46.5,7.25,1200,270,1",
        "46.5,7.25,NaN,270",
        "91,7.25,1200,270",
        "46.5,181,1200,270",
        "46.5,7.25,1200,360",
        "46.5,7.25,1200,-1",
    ] {
        let Err(XtaskError::Usage { message }) = Site::parse(spec) else {
            panic!("{spec:?} must be refused");
        };
        assert!(message.contains("n51, zurich"), "{spec:?}: {message}");
    }
}

#[test]
fn a_runway_vehicle_starts_on_the_threshold_facing_down_the_runway() {
    let plane = N51.spawn_point(Ground::Runway, 0);
    assert_eq!(
        plane,
        SpawnPoint {
            latitude_deg: N51.latitude_deg,
            longitude_deg: N51.longitude_deg,
            elevation_m: 56.3,
            heading_deg: 29.5,
        }
    );
    let (along, left) = along_left_m(&N51, &N51.spawn_point(Ground::Runway, 1));
    assert!(
        (along - 60.0).abs() < 0.2 && left.abs() < 0.2,
        "{along} {left}"
    );
}

/// The N51 asphalt is 15 m wide; its taxiway and hangars are on the
/// right. The grass row stands 40 m to the left, beside the runway and
/// past the threshold, and each quad faces down the runway too.
#[test]
fn field_vehicles_stand_in_a_row_on_the_grass_left_of_the_runway() {
    for index in 0..5 {
        let point = N51.spawn_point(Ground::Field, index);
        let (along, left) = along_left_m(&N51, &point);
        let expected_along = 40.0 * (index as f64 + 1.0);
        assert!((along - expected_along).abs() < 0.2, "{index}: {along} m");
        assert!((left - 40.0).abs() < 0.2, "{index}: {left} m left");
        assert_eq!(point.heading_deg, N51.heading_deg);
        assert_eq!(point.elevation_m, N51.elevation_m);
    }
}
