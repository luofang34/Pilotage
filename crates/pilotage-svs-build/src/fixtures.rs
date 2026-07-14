//! Deterministic, synthetic fixtures for the tests.
//!
//! Everything here is small and made up: a four-by-four terrain grid, a handful
//! of obstacles, one aerodrome with a runway, all on WGS-84 with ellipsoidal
//! heights. Keys and seeds are fixed, so every build is reproducible with no
//! randomness and no I/O. No real or license-restricted data is used.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::{
    DatumRealizationId, GeoidModelId, HorizontalDatum, IntegrityLevel, VerticalDatum,
};
use pilotage_svs_db::{
    Accuracy, CoverageBox, DatasetId, DayNumber, Effectivity, PackageVersion, ProviderId,
    TrustKeyId, UseRestrictions,
};

use crate::config::{BuildConfig, ChainParams, PackageIdentity, SigningConfig, TargetDatum};
use crate::source::{
    Aerodrome, LicenseCode, Obstacle, ObstacleKind, Runway, SourceDataset, SourceId, SourceMeta,
    SourceRecordRef, TerrainGrid,
};

/// The terrain source id.
pub(crate) const TERRAIN_SRC: SourceId = SourceId(1);
/// The obstacle source id.
pub(crate) const OBSTACLE_SRC: SourceId = SourceId(2);
/// The aerodrome source id.
pub(crate) const AERODROME_SRC: SourceId = SourceId(3);

/// A fixed, valid build configuration on WGS-84 / ellipsoid.
pub(crate) fn config() -> BuildConfig {
    BuildConfig {
        identity: PackageIdentity {
            dataset: DatasetId(1),
            provider: ProviderId(0xB2),
            version: PackageVersion::new(1, 0, 0),
            effectivity: Effectivity {
                release: DayNumber(100),
                effective: DayNumber(100),
                expiry: DayNumber(200),
            },
            simulation_only: true,
            base_restrictions: UseRestrictions::NO_OPERATIONAL_USE,
        },
        coverage: CoverageBox {
            min_lat_deg: 40.0,
            max_lat_deg: 41.0,
            min_lon_deg: -75.0,
            max_lon_deg: -74.0,
        },
        target: TargetDatum {
            horizontal: HorizontalDatum::Wgs84,
            realization: DatumRealizationId::UNDECLARED,
            vertical: VerticalDatum::Ellipsoid,
            geoid: GeoidModelId::UNDECLARED,
        },
        params: ChainParams {
            tile_deg: 0.5,
            post_spacing_deg: 0.25,
            post_spacing_mm: 30_000,
            elevation_min_m: -500.0,
            elevation_max_m: 9_000.0,
            max_obstacle_height_m: 1_000.0,
            max_hole_span: 2,
            merge_tolerance_deg: 0.001,
            integrity: IntegrityLevel::Monitored,
        },
        signing: SigningConfig {
            key_id: TrustKeyId(0xA1),
            signing_seed: [7u8; 32],
        },
    }
}

/// Metadata for a WGS-84 / ellipsoid source under the given license.
pub(crate) fn meta(id: SourceId, license: LicenseCode) -> SourceMeta {
    SourceMeta {
        id,
        version: 1,
        license,
        horizontal_datum: HorizontalDatum::Wgs84,
        realization: DatumRealizationId::UNDECLARED,
        vertical_datum: VerticalDatum::Ellipsoid,
        geoid: GeoidModelId::UNDECLARED,
        accuracy: Accuracy {
            horizontal_mm: 3_000,
            vertical_mm: 5_000,
        },
    }
}

/// A clean four-by-four terrain grid bracketing the coverage box.
pub(crate) fn terrain_grid() -> TerrainGrid {
    let (rows, cols) = (4u32, 4u32);
    let mut posts = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            posts.push(Some(100.0 + f64::from(r) * 10.0 + f64::from(c)));
        }
    }
    TerrainGrid {
        source: TERRAIN_SRC,
        origin_lat_deg: 39.5,
        origin_lon_deg: -75.5,
        step_deg: 0.5,
        rows,
        cols,
        posts,
    }
}

/// One obstacle inside coverage.
pub(crate) fn obstacle() -> Obstacle {
    Obstacle {
        lat_deg: 40.2,
        lon_deg: -74.7,
        height_m: 50.0,
        kind: ObstacleKind::Tower,
        source: SourceRecordRef {
            source: OBSTACLE_SRC,
            record: 0,
        },
    }
}

/// One aerodrome with a single runway, inside coverage.
pub(crate) fn aerodrome() -> Aerodrome {
    Aerodrome {
        ident: 0x4B50_5859,
        ref_lat_deg: 40.5,
        ref_lon_deg: -74.5,
        elevation_m: 100.0,
        source: SourceRecordRef {
            source: AERODROME_SRC,
            record: 0,
        },
        runways: vec![Runway {
            designator: 0x0918,
            end_a_lat_deg: 40.49,
            end_a_lon_deg: -74.51,
            end_b_lat_deg: 40.51,
            end_b_lon_deg: -74.49,
            source: SourceRecordRef {
                source: AERODROME_SRC,
                record: 1,
            },
        }],
    }
}

/// The clean dataset: terrain, one obstacle, one aerodrome with a runway.
pub(crate) fn dataset() -> SourceDataset {
    SourceDataset {
        meta: vec![
            meta(TERRAIN_SRC, LicenseCode::Open),
            meta(OBSTACLE_SRC, LicenseCode::Open),
            meta(AERODROME_SRC, LicenseCode::Open),
        ],
        terrain: vec![terrain_grid()],
        obstacles: vec![obstacle()],
        aerodromes: vec![aerodrome()],
    }
}
