//! Edge-case tests for datum discipline (anti-meridian, polar, tile-seam,
//! vertical datum).
#![allow(clippy::expect_used, clippy::panic)]

use super::{
    GeodeticPosition, GeoidModelId, HorizontalDatum, LocalOriginId, VerticalDatum,
    VerticalPosition, wrap_longitude_deg,
};
use crate::error::GeoError;

fn ellipsoid(height_m: f64) -> VerticalPosition {
    VerticalPosition::new(
        height_m,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoid height is well-formed")
}

fn pos(lat: f64, lon: f64) -> GeodeticPosition {
    GeodeticPosition::new(lat, lon, HorizontalDatum::Wgs84, ellipsoid(100.0))
        .expect("well-formed geodetic position")
}

#[test]
fn longitude_wraps_to_canonical_half_open_range() {
    assert_eq!(wrap_longitude_deg(180.0), -180.0);
    assert_eq!(wrap_longitude_deg(-180.0), -180.0);
    assert_eq!(wrap_longitude_deg(181.0), -179.0);
    assert_eq!(wrap_longitude_deg(-181.0), 179.0);
    assert_eq!(wrap_longitude_deg(540.0), -180.0);
    assert_eq!(wrap_longitude_deg(0.0), 0.0);
}

#[test]
fn antimeridian_has_one_canonical_spelling() {
    let east = pos(0.0, 180.0);
    let west = pos(0.0, -180.0);
    assert_eq!(
        east.longitude_deg, west.longitude_deg,
        "±180 collapse to one"
    );
    assert!(east.on_antimeridian() && west.on_antimeridian());
    assert_eq!(east, west, "the two spellings are the same position");
}

#[test]
fn poles_are_valid_and_flagged() {
    let north = pos(90.0, 45.0);
    let south = pos(-90.0, -73.0);
    assert!(north.at_pole() && south.at_pole());
    // Longitude is retained deterministically even though it is degenerate.
    assert_eq!(north.longitude_deg, 45.0);
}

#[test]
fn latitude_out_of_range_is_refused() {
    let err = GeodeticPosition::new(90.001, 0.0, HorizontalDatum::Wgs84, ellipsoid(0.0))
        .expect_err("beyond the pole is refused");
    assert!(matches!(err, GeoError::LatitudeOutOfRange { .. }));
}

#[test]
fn tile_seam_belongs_to_exactly_one_tile() {
    // A position exactly on a 1-degree seam floors into the higher tile, and a
    // hair below floors into the lower one — deterministic, never oscillating.
    assert_eq!(pos(0.5, 1.0).tile(1.0).lon_index, 1);
    assert_eq!(pos(0.5, 0.999_999).tile(1.0).lon_index, 0);
    // The anti-meridian seam maps consistently (normalized to -180).
    assert_eq!(pos(0.5, 180.0).tile(1.0).lon_index, -180);
    assert_eq!(pos(0.5, -180.0).tile(1.0).lon_index, -180);
}

#[test]
fn unknown_vertical_datum_is_refused() {
    let err = VerticalPosition::new(
        10.0,
        VerticalDatum::Unknown,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect_err("an unknown datum has no interpretable reference");
    assert!(matches!(err, GeoError::UnknownVerticalDatum));
}

#[test]
fn geometric_msl_requires_a_declared_geoid() {
    let missing = VerticalPosition::new(
        10.0,
        VerticalDatum::Msl,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect_err("MSL without a geoid model is undefined");
    assert!(matches!(missing, GeoError::UndeclaredGeoidModel));

    VerticalPosition::new(
        10.0,
        VerticalDatum::Msl,
        GeoidModelId(12),
        LocalOriginId::UNDECLARED,
    )
    .expect("MSL with a declared geoid model is well-formed");
}

#[test]
fn local_relative_requires_a_declared_origin() {
    let missing = VerticalPosition::new(
        10.0,
        VerticalDatum::LocalRelative,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect_err("a relative height without an origin is a silent rebase risk");
    assert!(matches!(missing, GeoError::NonFinite { .. }));

    VerticalPosition::new(
        10.0,
        VerticalDatum::LocalRelative,
        GeoidModelId::UNDECLARED,
        LocalOriginId(7),
    )
    .expect("local-relative with a declared origin is well-formed");
}

#[test]
fn non_finite_coordinates_are_refused() {
    assert!(matches!(
        GeodeticPosition::new(f64::NAN, 0.0, HorizontalDatum::Wgs84, ellipsoid(0.0)),
        Err(GeoError::NonFinite { .. })
    ));
    assert!(matches!(
        ellipsoid_result(f64::INFINITY),
        Err(GeoError::NonFinite { .. })
    ));
}

fn ellipsoid_result(height_m: f64) -> Result<VerticalPosition, GeoError> {
    VerticalPosition::new(
        height_m,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
}

#[test]
fn unknown_horizontal_datum_is_refused() {
    let err = GeodeticPosition::new(0.0, 0.0, HorizontalDatum::Unknown, ellipsoid(0.0))
        .expect_err("an unknown horizontal datum has no interpretable frame");
    assert!(matches!(err, GeoError::UnknownVerticalDatum));
}

#[test]
fn datum_wire_codes_round_trip_and_reject_unknown() {
    for d in [
        VerticalDatum::Ellipsoid,
        VerticalDatum::Msl,
        VerticalDatum::Agl,
        VerticalDatum::BaroIndicated,
        VerticalDatum::Pressure,
        VerticalDatum::LocalRelative,
    ] {
        assert_eq!(VerticalDatum::from_u8(d.to_u8()), Some(d));
    }
    assert_eq!(
        VerticalDatum::from_u8(200),
        None,
        "unknown wire value fails closed"
    );
    assert_eq!(HorizontalDatum::from_u8(9), None);
}
