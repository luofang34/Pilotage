//! The world: the site's ground comes from the terrain where it covers
//! the site, placements stand on the terrain, and the magnetic field is
//! the published WMM2025 field.

#![allow(clippy::expect_used, clippy::panic)]

use super::{ElevationOrigin, World};
use crate::error::WorldError;
use crate::geomag::{MagneticEpoch, WMM2025};
use crate::site::{Ground, N51};
use crate::terrain::{UNIFORM_TERRAIN_ID, UniformTerrain};

fn epoch() -> MagneticEpoch {
    MagneticEpoch::from_decimal_year(2026.5)
}

#[test]
fn a_world_names_its_sources() {
    let world = World::new(N51, UniformTerrain { elevation_m: 0.0 }, epoch());
    let sources = world.sources();
    assert_eq!(sources.terrain, UNIFORM_TERRAIN_ID);
    assert_eq!(sources.magnetic_model, WMM2025);
    assert_eq!(sources.epoch, epoch());
}

/// Rows of NOAA's WMM2025 test-value file (WMM2025COF.zip,
/// `WMM2025_TestValues.txt`): decimal year, height km, latitude,
/// longitude, declination °, inclination °, X, Y, Z nT.
const NOAA_ROWS: [[f64; 9]; 3] = [
    [
        2025.0,
        28.0,
        89.0,
        -121.0,
        -99.77,
        88.47,
        -255.388723,
        -1482.460628,
        56194.288771,
    ],
    [
        2025.0,
        51.0,
        -33.0,
        109.0,
        -5.49,
        -67.50,
        21737.778822,
        -2090.274098,
        -52710.003920,
    ],
    [
        2025.5,
        63.0,
        26.0,
        81.0,
        0.51,
        41.07,
        34802.613433,
        308.431881,
        30332.056989,
    ],
];

#[test]
fn the_field_is_the_published_wmm2025_field() {
    for [year, height_km, lat, lon, decl, incl, x, y, z] in NOAA_ROWS {
        let world = World::new(
            N51,
            UniformTerrain { elevation_m: 0.0 },
            MagneticEpoch::from_decimal_year(year),
        );
        let field = world
            .magnetic_field(lat, lon, height_km * 1_000.0)
            .expect("inside the model");
        assert!((field.declination_deg - decl).abs() <= 0.01, "{field:?}");
        assert!((field.inclination_deg - incl).abs() <= 0.01, "{field:?}");
        for (got, want) in [(field.north_nt, x), (field.east_nt, y), (field.down_nt, z)] {
            assert!((got - want).abs() <= 0.001, "{got} vs {want}");
        }
    }
}

#[test]
fn n51_has_a_westerly_declination_and_a_steep_dip() {
    let world = World::new(N51, UniformTerrain { elevation_m: 56.3 }, epoch());
    let field = world
        .magnetic_field(N51.latitude_deg, N51.longitude_deg, 56.3)
        .expect("inside the model");
    // New Jersey: about 12° W, dip about 66° down.
    assert!(
        (-14.0..=-11.0).contains(&field.declination_deg),
        "{field:?}"
    );
    assert!((64.0..=68.0).contains(&field.inclination_deg), "{field:?}");
}

/// The world's air is the shared standard atmosphere at the world's MSL
/// height.
#[test]
fn the_air_is_the_standard_atmosphere() {
    let world = World::new(N51, UniformTerrain { elevation_m: 56.3 }, epoch());
    let air = world
        .atmosphere(N51.latitude_deg, N51.longitude_deg, 56.3)
        .expect("inside the model");
    let isa = world_atmosphere::Isa
        .at(world_atmosphere::Altitude::GeometricMsl(56.3))
        .expect("inside the model");
    assert_eq!(air, isa);
    // About 100.65 kPa at N51's field elevation.
    assert!((air.pressure_pa - 100_650.0).abs() < 30.0, "{air:?}");
    assert!(world.atmosphere(0.0, 0.0, 200_000.0).is_err());
}

#[test]
fn an_epoch_outside_the_model_is_refused() {
    let world = World::new(
        N51,
        UniformTerrain { elevation_m: 0.0 },
        MagneticEpoch::from_decimal_year(2031.0),
    );
    assert!(matches!(
        world.magnetic_field(N51.latitude_deg, N51.longitude_deg, 0.0),
        Err(WorldError::MagneticField { .. })
    ));
}

#[test]
fn a_uniform_world_puts_every_site_on_its_height() {
    let mut world = World::new(N51, UniformTerrain { elevation_m: 3.0 }, epoch());
    let elevation = world.site_elevation_blocking().expect("site elevation");
    assert_eq!(elevation.elevation_m, 3.0);
    assert!(matches!(elevation.origin, ElevationOrigin::Terrain(_)));
    let placement = world
        .placement_blocking(Ground::Field, 2)
        .expect("placement");
    assert_eq!(placement.elevation_m, 3.0);
    let site_placement = N51.placement(Ground::Field, 2).expect("placed");
    assert_eq!(placement.latitude_deg, site_placement.latitude_deg);
    assert_eq!(placement.longitude_deg, site_placement.longitude_deg);
}

#[cfg(feature = "archive")]
mod archive {
    use super::{ElevationOrigin, World, epoch};
    use crate::fixture::{Tile, archive};
    use crate::site::{Ground, N51};
    use crate::terrain::{ArchiveTerrain, TerrainSource};

    /// The site's ground comes from the terrain: a fixture whose tile
    /// over N51 stands at 61.25 m, not the declared 56.3 m.
    #[test]
    fn the_site_elevation_is_the_terrain_height_where_it_is_covered() {
        let file = archive(
            "n51 fixture",
            &[Tile::covering(
                15,
                N51.latitude_deg,
                N51.longitude_deg,
                61.25,
            )],
        );
        let terrain = ArchiveTerrain::open_blocking(file.path()).expect("open archive");
        let id = terrain.id();
        let mut world = World::new(N51, terrain, epoch());
        let elevation = world.site_elevation_blocking().expect("site elevation");
        assert!((elevation.elevation_m - 61.25).abs() < 1.0 / 256.0);
        let ElevationOrigin::Terrain(sample) = elevation.origin else {
            panic!("expected the terrain to supply the height");
        };
        assert_eq!(sample.source, id);
        assert_eq!(world.sources().terrain, id);
    }

    #[test]
    fn the_declared_height_stands_where_the_terrain_has_no_coverage() {
        let file = archive("elsewhere", &[Tile::covering(15, 47.4, 8.5, 400.0)]);
        let terrain = ArchiveTerrain::open_blocking(file.path()).expect("open archive");
        let mut world = World::new(N51, terrain, epoch());
        let elevation = world.site_elevation_blocking().expect("site elevation");
        assert_eq!(elevation.elevation_m, N51.elevation_m);
        assert_eq!(elevation.origin, ElevationOrigin::Declared);
        let placement = world
            .placement_blocking(Ground::Runway, 0)
            .expect("placement");
        assert_eq!(placement.elevation_m, N51.elevation_m);
    }

    /// The declared N51 elevation is the Terrarium sample at the
    /// threshold. Against a real archive that covers N51
    /// (`PILOTAGE_TERRAIN_ARCHIVE`), the two agree.
    #[test]
    #[ignore = "needs a Terrarium archive covering N51 in PILOTAGE_TERRAIN_ARCHIVE"]
    fn the_declared_n51_elevation_agrees_with_the_terrain_archive() {
        let path = std::env::var_os("PILOTAGE_TERRAIN_ARCHIVE")
            .expect("set PILOTAGE_TERRAIN_ARCHIVE to a Terrarium MBTiles archive");
        let terrain = ArchiveTerrain::open_blocking(path).expect("open archive");
        let mut world = World::new(N51, terrain, epoch());
        let elevation = world.site_elevation_blocking().expect("site elevation");
        let ElevationOrigin::Terrain(sample) = elevation.origin else {
            panic!("the archive does not cover N51");
        };
        assert!(
            sample.resolution_m < 30.0,
            "the archive answers N51 at {} m a pixel",
            sample.resolution_m
        );
        assert!(
            (elevation.elevation_m - N51.elevation_m).abs() < 1.0,
            "terrain {} m, declared {} m",
            elevation.elevation_m,
            N51.elevation_m
        );
    }
}
