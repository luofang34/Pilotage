//! Bilinear sampling from source terrain grids into WGS-84 and MSL.

use pilotage_geo::{HorizontalDatum, VerticalDatum};
use pilotage_svs_build::{
    SourceDataset, SourceMeta, TerrainGrid, convert_horizontal, convert_vertical,
};

use crate::TerrainBuildError;

struct GridSource<'a> {
    grid: &'a TerrainGrid,
    meta: &'a SourceMeta,
}

pub(crate) struct TerrainSampler<'a> {
    grids: Vec<GridSource<'a>>,
}

impl<'a> TerrainSampler<'a> {
    pub(crate) fn new(source: &'a SourceDataset) -> Result<Self, TerrainBuildError> {
        if source.terrain.is_empty() {
            return Err(TerrainBuildError::EmptyTerrain);
        }
        let mut grids = Vec::with_capacity(source.terrain.len());
        for grid in &source.terrain {
            let meta = unique_meta(source, grid.source.0)?;
            validate_meta(meta)?;
            validate_grid(grid)?;
            grids.push(GridSource { grid, meta });
        }
        grids.sort_by(|a, b| {
            a.grid
                .step_deg
                .total_cmp(&b.grid.step_deg)
                .then(a.grid.source.0.cmp(&b.grid.source.0))
                .then(a.grid.origin_lat_deg.total_cmp(&b.grid.origin_lat_deg))
                .then(a.grid.origin_lon_deg.total_cmp(&b.grid.origin_lon_deg))
        });
        Ok(Self { grids })
    }

    pub(crate) fn sample_msl(
        &self,
        lat_deg: f64,
        lon_deg: f64,
    ) -> Result<Option<f64>, TerrainBuildError> {
        for source in &self.grids {
            if let Some(height_m) = sample_grid(source, lat_deg, lon_deg)? {
                return Ok(Some(height_m));
            }
        }
        Ok(None)
    }
}

fn unique_meta(source: &SourceDataset, source_id: u32) -> Result<&SourceMeta, TerrainBuildError> {
    let mut matching = source.meta.iter().filter(|meta| meta.id.0 == source_id);
    let meta = matching
        .next()
        .ok_or(TerrainBuildError::MissingSourceMetadata { source_id })?;
    if matching.next().is_some() {
        return Err(TerrainBuildError::DuplicateSourceMetadata { source_id });
    }
    Ok(meta)
}

fn validate_meta(meta: &SourceMeta) -> Result<(), TerrainBuildError> {
    let source_id = meta.id.0;
    if meta.horizontal_datum == HorizontalDatum::Unknown {
        return Err(TerrainBuildError::UnsupportedSourceDatum {
            source_id,
            axis: "horizontal",
            code: meta.horizontal_datum.to_u8(),
        });
    }
    if meta.horizontal_datum.needs_realization() && !meta.realization.is_declared() {
        return Err(TerrainBuildError::IncompleteSourceDatum {
            source_id,
            reason: "horizontal realization is not declared",
        });
    }
    if !matches!(
        meta.vertical_datum,
        VerticalDatum::Msl | VerticalDatum::Ellipsoid
    ) {
        return Err(TerrainBuildError::UnsupportedSourceDatum {
            source_id,
            axis: "vertical",
            code: meta.vertical_datum.to_u8(),
        });
    }
    if meta.vertical_datum == VerticalDatum::Msl && !meta.geoid.is_declared() {
        return Err(TerrainBuildError::IncompleteSourceDatum {
            source_id,
            reason: "MSL geoid model is not declared",
        });
    }
    Ok(())
}

fn validate_grid(grid: &TerrainGrid) -> Result<(), TerrainBuildError> {
    let reason = if !(grid.step_deg.is_finite() && grid.step_deg > 0.0) {
        Some("step must be positive and finite")
    } else if grid.rows < 2 || grid.cols < 2 {
        Some("grid must have at least two rows and columns")
    } else if !(grid.origin_lat_deg.is_finite() && grid.origin_lon_deg.is_finite()) {
        Some("origin must be finite")
    } else if grid.posts.len() != (grid.rows as usize).saturating_mul(grid.cols as usize) {
        Some("post count must equal rows times columns")
    } else if grid
        .posts
        .iter()
        .flatten()
        .any(|height| !height.is_finite())
    {
        Some("present posts must be finite")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(TerrainBuildError::InvalidTerrainGrid {
            source_id: grid.source.0,
            reason,
        }),
        None => Ok(()),
    }
}

fn sample_grid(
    source: &GridSource<'_>,
    lat_wgs84: f64,
    lon_wgs84: f64,
) -> Result<Option<f64>, TerrainBuildError> {
    let source_id = source.grid.source.0;
    let (lat, lon) = convert_horizontal(
        lat_wgs84,
        lon_wgs84,
        HorizontalDatum::Wgs84,
        source.meta.horizontal_datum,
    )
    .map_err(|error| TerrainBuildError::DatumConversion {
        source_id,
        source: error,
    })?;
    let Some(height) = interpolate(source.grid, lat, lon) else {
        return Ok(None);
    };
    convert_vertical(
        height,
        source.meta.vertical_datum,
        VerticalDatum::Msl,
        lat_wgs84,
        lon_wgs84,
    )
    .map(Some)
    .map_err(|error| TerrainBuildError::DatumConversion {
        source_id,
        source: error,
    })
}

fn interpolate(grid: &TerrainGrid, lat_deg: f64, lon_deg: f64) -> Option<f64> {
    let row = (lat_deg - grid.origin_lat_deg) / grid.step_deg;
    let col = (lon_deg - grid.origin_lon_deg) / grid.step_deg;
    if !(row.is_finite() && col.is_finite() && row >= 0.0 && col >= 0.0) {
        return None;
    }
    let r0f = row.floor();
    let c0f = col.floor();
    if r0f + 1.0 >= f64::from(grid.rows) || c0f + 1.0 >= f64::from(grid.cols) {
        return None;
    }
    let r0 = r0f as u32;
    let c0 = c0f as u32;
    let h00 = grid.post(r0, c0)?;
    let h01 = grid.post(r0, c0 + 1)?;
    let h10 = grid.post(r0 + 1, c0)?;
    let h11 = grid.post(r0 + 1, c0 + 1)?;
    let row_fraction = row - r0f;
    let col_fraction = col - c0f;
    let south = h00 * (1.0 - col_fraction) + h01 * col_fraction;
    let north = h10 * (1.0 - col_fraction) + h11 * col_fraction;
    Some(south * (1.0 - row_fraction) + north * row_fraction)
}
