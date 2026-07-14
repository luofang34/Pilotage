//! Unit tests for coordinate and vertical-datum conversion.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::{HorizontalDatum, VerticalDatum};

use super::{convert_horizontal, convert_vertical, geoid_separation_m};
use crate::error::BuildError;

#[test]
fn same_horizontal_datum_is_identity() {
    let (lat, lon) =
        convert_horizontal(40.0, -74.0, HorizontalDatum::Wgs84, HorizontalDatum::Wgs84).unwrap();
    assert_eq!((lat, lon), (40.0, -74.0));
}

#[test]
fn nad83_to_wgs84_shifts_and_inverts() {
    let (lat, lon) =
        convert_horizontal(40.0, -74.0, HorizontalDatum::Nad83, HorizontalDatum::Wgs84).unwrap();
    let (back_lat, back_lon) =
        convert_horizontal(lat, lon, HorizontalDatum::Wgs84, HorizontalDatum::Nad83).unwrap();
    assert!((back_lat - 40.0).abs() < 1e-12);
    assert!((back_lon - (-74.0)).abs() < 1e-12);
    assert_ne!(
        lat, 40.0,
        "the NAD83->WGS84 shift must actually move latitude"
    );
}

#[test]
fn unknown_horizontal_datum_is_refused() {
    let result = convert_horizontal(
        40.0,
        -74.0,
        HorizontalDatum::Unknown,
        HorizontalDatum::Wgs84,
    );
    assert!(matches!(
        result,
        Err(BuildError::UnsupportedDatumConversion {
            axis: "horizontal",
            ..
        })
    ));
}

#[test]
fn horizontal_conversion_wraps_longitude() {
    let (_, lon) =
        convert_horizontal(0.0, 180.0, HorizontalDatum::Wgs84, HorizontalDatum::Wgs84).unwrap();
    assert_eq!(lon, -180.0, "the anti-meridian normalizes to -180");
}

#[test]
fn same_vertical_datum_is_identity() {
    let h = convert_vertical(123.0, VerticalDatum::Msl, VerticalDatum::Msl, 40.0, -74.0).unwrap();
    assert_eq!(h, 123.0);
}

#[test]
fn msl_to_ellipsoid_adds_separation_and_inverts() {
    let n = geoid_separation_m(40.0, -74.0);
    let ellip = convert_vertical(
        100.0,
        VerticalDatum::Msl,
        VerticalDatum::Ellipsoid,
        40.0,
        -74.0,
    )
    .unwrap();
    assert!((ellip - (100.0 + n)).abs() < 1e-12);
    let back = convert_vertical(
        ellip,
        VerticalDatum::Ellipsoid,
        VerticalDatum::Msl,
        40.0,
        -74.0,
    )
    .unwrap();
    assert!((back - 100.0).abs() < 1e-9);
}

#[test]
fn unsupported_vertical_conversion_is_refused() {
    let result = convert_vertical(10.0, VerticalDatum::Agl, VerticalDatum::Ellipsoid, 0.0, 0.0);
    assert!(matches!(
        result,
        Err(BuildError::UnsupportedDatumConversion {
            axis: "vertical",
            ..
        })
    ));
}

#[test]
fn geoid_separation_is_deterministic() {
    assert_eq!(
        geoid_separation_m(12.5, -33.0),
        geoid_separation_m(12.5, -33.0)
    );
}
