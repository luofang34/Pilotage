//! The ground grid: sampled from the world, exact on a plane, clamped
//! beyond its edge, and re-centred as the vehicle flies away.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use pilotage_geo::TerrainRefId;
use pilotage_world::{
    MagneticEpoch, N51, Ned, TerrainError, TerrainSample, TerrainSource, UniformTerrain, World,
    local_plane, world_position,
};

use super::{GroundModel, GroundPatch};

/// North and east metres of a point from the N51 threshold, in the world
/// model's frame there.
fn offset(latitude_deg: f64, longitude_deg: f64) -> (f64, f64) {
    let plane = local_plane(N51.latitude_deg, N51.longitude_deg, N51.elevation_m).unwrap();
    let ned = plane
        .to_ned(world_position(latitude_deg, longitude_deg, N51.elevation_m))
        .unwrap();
    (ned.north_m, ned.east_m)
}

/// The point `north_m` and `east_m` from the N51 threshold.
fn point(north_m: f64, east_m: f64) -> (f64, f64) {
    let plane = local_plane(N51.latitude_deg, N51.longitude_deg, N51.elevation_m).unwrap();
    let at = plane.to_geodetic(Ned {
        north_m,
        east_m,
        down_m: 0.0,
    });
    (at.latitude_deg, at.longitude_deg)
}

/// A plane rising 0.1 m per metre north and falling 0.05 m per metre
/// east of the N51 threshold, from 50 m there.
struct Slope;

impl TerrainSource for Slope {
    fn id(&self) -> TerrainRefId {
        TerrainRefId(7)
    }

    fn sample_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<TerrainSample>, TerrainError> {
        let (north, east) = offset(latitude_deg, longitude_deg);
        let mut sample = UniformTerrain {
            elevation_m: 50.0 + 0.1 * north - 0.05 * east,
        }
        .sample_blocking(latitude_deg, longitude_deg)?
        .unwrap();
        sample.resolution_m = 12.0;
        Ok(Some(sample))
    }
}

fn world<T: TerrainSource>(terrain: T) -> World<T> {
    World::new(N51, terrain, MagneticEpoch::from_decimal_year(2026.5))
}

#[test]
fn a_uniform_world_is_flat_everywhere_on_the_grid() {
    let mut world = world(UniformTerrain { elevation_m: 56.3 });
    let patch =
        GroundPatch::sample_blocking(&mut world, N51.latitude_deg, N51.longitude_deg, 4).unwrap();
    assert_eq!(patch.spacing_m(), 10.0);
    let answer = patch.answer(N51.latitude_deg + 0.0001, N51.longitude_deg - 0.0002);
    assert_eq!(answer.height_m, 56.3);
    assert_eq!((answer.slope_north, answer.slope_east), (0.0, 0.0));
}

#[test]
fn a_plane_is_answered_exactly_between_grid_points() {
    let mut world = world(Slope);
    let patch =
        GroundPatch::sample_blocking(&mut world, N51.latitude_deg, N51.longitude_deg, 8).unwrap();
    assert_eq!(patch.spacing_m(), 12.0);
    for (north, east) in [(0.0, 0.0), (7.3, -3.1), (-40.0, 55.5), (90.0, 90.0)] {
        let (lat, lon) = point(north, east);
        let answer = patch.answer(lat, lon);
        let expected = 50.0 + 0.1 * north - 0.05 * east;
        assert!(
            (answer.height_m - expected).abs() < 1e-6,
            "{north} {east}: {answer:?}"
        );
        assert!((answer.slope_north - 0.1).abs() < 1e-9);
        assert!((answer.slope_east + 0.05).abs() < 1e-9);
    }
    // Through the callback, in radians.
    let through = patch
        .height(
            N51.latitude_deg.to_radians(),
            N51.longitude_deg.to_radians(),
        )
        .unwrap();
    assert!((through.height_m - 50.0).abs() < 1e-6);
}

#[test]
fn beyond_the_grid_the_edge_height_stands() {
    let mut world = world(Slope);
    let patch =
        GroundPatch::sample_blocking(&mut world, N51.latitude_deg, N51.longitude_deg, 2).unwrap();
    // The grid reaches 24 m north; 1 km north reads the edge, 52.4 m.
    let (lat, lon) = point(1_000.0, 0.0);
    let far = patch.answer(lat, lon);
    assert!((far.height_m - 52.4).abs() < 1e-6, "{far:?}");
}

#[test]
fn the_grid_is_resampled_past_half_its_reach() {
    let mut world = world(UniformTerrain { elevation_m: 0.0 });
    let patch =
        GroundPatch::sample_blocking(&mut world, N51.latitude_deg, N51.longitude_deg, 32).unwrap();
    // 32 cells of 10 m reach 320 m; re-centre past 160 m.
    let (lat, lon) = point(150.0, 0.0);
    assert!(!patch.needs_recentre(lat, lon));
    let (lat, lon) = point(170.0, 0.0);
    assert!(patch.needs_recentre(lat, lon));
    let (lat, lon) = point(0.0, -170.0);
    assert!(patch.needs_recentre(lat, lon));
}

/// The geodetic up direction at a point, in ECEF.
fn up(latitude_deg: f64, longitude_deg: f64) -> [f64; 3] {
    let (lat, lon) = (latitude_deg.to_radians(), longitude_deg.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn the_contact_normal_is_up_on_flat_ground_and_tilted_by_the_slope() {
    let mut flat = world(UniformTerrain { elevation_m: 56.3 });
    let patch =
        GroundPatch::sample_blocking(&mut flat, N51.latitude_deg, N51.longitude_deg, 4).unwrap();
    let normal = patch
        .answer(N51.latitude_deg, N51.longitude_deg)
        .normal_ecef;
    assert!(dot(normal, up(N51.latitude_deg, N51.longitude_deg)) > 1.0 - 1e-12);
    let mut hill = world(Slope);
    let patch =
        GroundPatch::sample_blocking(&mut hill, N51.latitude_deg, N51.longitude_deg, 4).unwrap();
    let normal = patch
        .answer(N51.latitude_deg, N51.longitude_deg)
        .normal_ecef;
    assert!((dot(normal, normal) - 1.0).abs() < 1e-12);
    let tilt = dot(normal, up(N51.latitude_deg, N51.longitude_deg)).acos();
    assert!((tilt - 0.1_f64.hypot(0.05).atan()).abs() < 1e-6, "{tilt}");
    // Rising to the north, the normal leans south: toward -z of north.
    let (lat, lon) = (
        N51.latitude_deg.to_radians(),
        N51.longitude_deg.to_radians(),
    );
    let north = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
    assert!(dot(normal, north) < 0.0);
}
