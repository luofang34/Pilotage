//! Terrain sources: a uniform world states its height, and an archive
//! answers with its own height, resolution, and identity.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_geo::{SIMULATOR_GEOID_MODEL_ID, VerticalDatum};

use super::{TerrainSource, UNIFORM_TERRAIN_ID, UniformTerrain};
use crate::error::TerrainError;

#[test]
fn a_uniform_world_states_one_msl_height_everywhere() {
    let mut terrain = UniformTerrain { elevation_m: 12.5 };
    for (lat, lon) in [(0.0, 0.0), (40.5, -74.7), (-60.0, 170.0)] {
        let sample = terrain
            .sample_blocking(lat, lon)
            .expect("uniform terrain answers")
            .expect("uniform terrain covers the world");
        assert_eq!(sample.elevation_m(), 12.5);
        assert_eq!(sample.height.datum, VerticalDatum::Msl);
        assert_eq!(sample.height.geoid, SIMULATOR_GEOID_MODEL_ID);
        assert_eq!(sample.source, UNIFORM_TERRAIN_ID);
        assert_eq!(sample.resolution_m, 0.0);
    }
}

#[test]
fn a_non_finite_height_is_refused_not_passed_on() {
    let mut terrain = UniformTerrain {
        elevation_m: f64::NAN,
    };
    assert!(matches!(
        terrain.sample_blocking(0.0, 0.0),
        Err(TerrainError::Height { .. })
    ));
}

#[cfg(feature = "archive")]
mod archive {
    use super::super::ArchiveTerrain;
    use super::super::archive::{archive_id, pixel_size_m};
    use super::{TerrainSource, UNIFORM_TERRAIN_ID};
    use crate::fixture::{TILE_SIZE, Tile, archive};

    #[test]
    fn the_deepest_tile_supplies_height_and_resolution() {
        let file = archive(
            "fixture",
            &[
                Tile::covering(0, 40.5, -74.7, 0.0),
                Tile::covering(15, 40.5, -74.7, 61.25),
            ],
        );
        let mut terrain = ArchiveTerrain::open_blocking(file.path()).expect("open archive");
        let sample = terrain
            .sample_blocking(40.5, -74.7)
            .expect("query")
            .expect("covered");
        assert!((sample.elevation_m() - 61.25).abs() < 1.0 / 256.0);
        assert_eq!(sample.source, terrain.id());
        assert_eq!(sample.resolution_m, pixel_size_m(40.5, 15, TILE_SIZE));
        let coarse = terrain
            .sample_blocking(10.0, 10.0)
            .expect("query")
            .expect("the world tile covers it");
        assert_eq!(coarse.elevation_m(), 0.0);
        assert!(coarse.resolution_m > 1.0e6, "{}", coarse.resolution_m);
    }

    #[test]
    fn the_pixel_size_is_the_web_mercator_ground_size() {
        // A 256-pixel tile at zoom 0 spans the equator: 156 543 m a pixel.
        assert!((pixel_size_m(0.0, 0, 256) - 156_543.03).abs() < 0.01);
        // Half as wide one zoom deeper, and narrower by cos(latitude).
        let n51 = pixel_size_m(40.577, 13, 256);
        assert!((n51 - 156_543.03 / 8192.0 * 40.577_f64.to_radians().cos()).abs() < 1e-6);
    }

    #[test]
    fn an_archive_identity_follows_its_name_and_version() {
        let a = archive_id("Pilotage situation terrain", Some("1"));
        assert_eq!(a, archive_id("Pilotage situation terrain", Some("1")));
        assert_ne!(a, archive_id("Pilotage situation terrain", Some("2")));
        assert_ne!(a, archive_id("Pilotage situation terrain", None));
        assert_ne!(archive_id("ab", Some("c")), archive_id("a", Some("bc")));
        for id in [a, archive_id("", None)] {
            assert!(id.is_declared());
            assert_ne!(id, UNIFORM_TERRAIN_ID);
        }
    }
}
