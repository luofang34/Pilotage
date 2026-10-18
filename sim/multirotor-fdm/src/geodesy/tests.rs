use super::Origin;
use crate::atmosphere::pressure_pa;

const ORIGIN: Origin = Origin {
    lat_deg: 47.397_742,
    lon_deg: 8.545_594,
    alt_m: 488.0,
};

#[test]
fn local_and_geodetic_round_trip() {
    for (n, e, d) in [(0.0, 0.0, 0.0), (35.0, -20.0, -12.5), (-400.0, 250.0, 3.0)] {
        let (lat, lon, alt) = ORIGIN.to_geodetic(n, e, d);
        let (n2, e2, d2) = ORIGIN.to_local(lat, lon, alt);
        assert!((n - n2).abs() < 1e-6 && (e - e2).abs() < 1e-6 && (d - d2).abs() < 1e-9);
    }
}

#[test]
fn a_degree_has_the_wgs84_length_at_the_origin_latitude() {
    let (north, _, _) = ORIGIN.to_local(ORIGIN.lat_deg + 1.0, ORIGIN.lon_deg, ORIGIN.alt_m);
    let (_, east, _) = ORIGIN.to_local(ORIGIN.lat_deg, ORIGIN.lon_deg + 1.0, ORIGIN.alt_m);
    // WGS-84 at 47.4°: 111.18 km per degree of latitude, 75.40 km of longitude.
    assert!((north - 111_180.0).abs() < 50.0, "{north}");
    assert!((east - 75_400.0).abs() < 100.0, "{east}");
}

#[test]
fn standard_pressure_falls_with_altitude() {
    assert!((pressure_pa(0.0) - 101_325.0).abs() < 1e-6);
    assert!((pressure_pa(1_000.0) - 89_875.0).abs() < 10.0);
    assert!(pressure_pa(489.0) < pressure_pa(488.0));
}
