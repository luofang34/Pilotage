//! Writes the synthetic situation-client terrain fixture.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pilotage_geo::{DatumRealizationId, GeoidModelId, HorizontalDatum, VerticalDatum};
use pilotage_svs_build::{Accuracy, LicenseCode, SourceDataset, SourceId, SourceMeta, TerrainGrid};
use pilotage_terrain_build::{
    TerrainBuildConfig, TerrainRegion, WEB_MERCATOR_MAX_LAT_DEG, build_mbtiles,
};

const SOURCE_ID: SourceId = SourceId(355);

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("an output path is required")]
    MissingOutputPath,
    #[error("the terrain fixture build failed")]
    Build(#[from] pilotage_terrain_build::TerrainBuildError),
    #[error("cannot write terrain fixture to {path:?}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn main() -> Result<(), FixtureError> {
    let output = output_path(std::env::args_os())?;
    let bundle = build_mbtiles(&source_dataset(), config())?;
    write_fixture_blocking(&output, bundle.bytes())
}

fn output_path(mut arguments: impl Iterator<Item = OsString>) -> Result<PathBuf, FixtureError> {
    arguments.next();
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or(FixtureError::MissingOutputPath)
}

fn write_fixture_blocking(path: &Path, bytes: &[u8]) -> Result<(), FixtureError> {
    std::fs::write(path, bytes).map_err(|source| FixtureError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn config() -> TerrainBuildConfig {
    TerrainBuildConfig {
        region: TerrainRegion {
            min_lat_deg: -WEB_MERCATOR_MAX_LAT_DEG,
            max_lat_deg: WEB_MERCATOR_MAX_LAT_DEG,
            min_lon_deg: -180.0,
            max_lon_deg: 180.0,
        },
        min_zoom: 0,
        max_zoom: 0,
    }
}

fn source_dataset() -> SourceDataset {
    let rows = 3u32;
    let cols = 5u32;
    SourceDataset {
        meta: vec![source_meta()],
        terrain: vec![TerrainGrid {
            source: SOURCE_ID,
            origin_lat_deg: -90.0,
            origin_lon_deg: -180.0,
            step_deg: 90.0,
            rows,
            cols,
            posts: terrain_posts(rows, cols),
        }],
        obstacles: Vec::new(),
        aerodromes: Vec::new(),
    }
}

fn source_meta() -> SourceMeta {
    SourceMeta {
        id: SOURCE_ID,
        version: 1,
        license: LicenseCode::Open,
        horizontal_datum: HorizontalDatum::Wgs84,
        realization: DatumRealizationId::UNDECLARED,
        vertical_datum: VerticalDatum::Msl,
        geoid: GeoidModelId(355),
        accuracy: Accuracy {
            horizontal_mm: 10_000_000,
            vertical_mm: 10_000,
        },
    }
}

fn terrain_posts(rows: u32, cols: u32) -> Vec<Option<f64>> {
    let mut posts = Vec::with_capacity((rows * cols) as usize);
    for row in 0..rows {
        for col in 0..cols {
            posts.push(Some(100.0 + f64::from(row * 20 + col * 3)));
        }
    }
    posts
}
