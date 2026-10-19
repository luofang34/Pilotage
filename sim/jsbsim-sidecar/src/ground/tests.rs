//! The ground grid: sampled from the world, exact on a plane, clamped
//! beyond its edge, and re-centred as the vehicle flies away.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use pilotage_geo::TerrainRefId;
use pilotage_world::{
    MagneticEpoch, N51, TerrainError, TerrainSample, TerrainSource, UniformTerrain, World,
    metres_per_degree,
};

use super::{GroundModel, GroundPatch};

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
        let (per_lat, per_lon) = metres_per_degree(N51.latitude_deg);
        let north = (latitude_deg - N51.latitude_deg) * per_lat;
        let east = (longitude_deg - N51.longitude_deg) * per_lon;
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
    let (per_lat, per_lon) = metres_per_degree(N51.latitude_deg);
    for (north, east) in [(0.0, 0.0), (7.3, -3.1), (-40.0, 55.5), (90.0, 90.0)] {
        let answer = patch.answer(
            N51.latitude_deg + north / per_lat,
            N51.longitude_deg + east / per_lon,
        );
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
    let (per_lat, _) = metres_per_degree(N51.latitude_deg);
    // The grid reaches 24 m north; 1 km north reads the edge, 52.4 m.
    let far = patch.answer(N51.latitude_deg + 1_000.0 / per_lat, N51.longitude_deg);
    assert!((far.height_m - 52.4).abs() < 1e-6, "{far:?}");
}

#[test]
fn the_grid_is_resampled_past_half_its_reach() {
    let mut world = world(UniformTerrain { elevation_m: 0.0 });
    let patch =
        GroundPatch::sample_blocking(&mut world, N51.latitude_deg, N51.longitude_deg, 32).unwrap();
    let (per_lat, per_lon) = metres_per_degree(N51.latitude_deg);
    // 32 cells of 10 m reach 320 m; re-centre past 160 m.
    assert!(!patch.needs_recentre(N51.latitude_deg + 150.0 / per_lat, N51.longitude_deg));
    assert!(patch.needs_recentre(N51.latitude_deg + 170.0 / per_lat, N51.longitude_deg));
    assert!(patch.needs_recentre(N51.latitude_deg, N51.longitude_deg - 170.0 / per_lon));
}
