//! WGS84 to Web Mercator tile coordinates.

use aerocontext_core::GeoPoint;

use crate::NavdataTileError;

pub(crate) const EXTENT: i32 = 4096;
pub(crate) const MAX_LATITUDE: f64 = 85.051_128_779_806_6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WorldPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TileCoord {
    pub(crate) zoom: u8,
    pub(crate) x: u32,
    pub(crate) y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TileBounds {
    pub(crate) min_x: f64,
    pub(crate) min_y: f64,
    pub(crate) max_x: f64,
    pub(crate) max_y: f64,
}

pub(crate) fn project(identifier: &str, point: GeoPoint) -> Result<WorldPoint, NavdataTileError> {
    if !point.lat.is_finite()
        || !point.lon.is_finite()
        || !(-90.0..=90.0).contains(&point.lat)
        || !(-180.0..=180.0).contains(&point.lon)
    {
        return Err(NavdataTileError::InvalidCoordinate {
            identifier: identifier.to_owned(),
            latitude: point.lat,
            longitude: point.lon,
        });
    }
    let latitude = point.lat.clamp(-MAX_LATITUDE, MAX_LATITUDE);
    let sin_latitude = latitude.to_radians().sin();
    let x = (point.lon + 180.0) / 360.0;
    let y = 0.5 - ((1.0 + sin_latitude) / (1.0 - sin_latitude)).ln() / (4.0 * std::f64::consts::PI);
    Ok(WorldPoint {
        x: x.clamp(0.0, 1.0),
        y: y.clamp(0.0, 1.0),
    })
}

pub(crate) fn tile_for_point(point: WorldPoint, zoom: u8) -> TileCoord {
    let width = matrix_width(zoom);
    let x = matrix_index(point.x, width);
    let y = matrix_index(point.y, width);
    TileCoord { zoom, x, y }
}

pub(crate) fn tiles_for_bounds(
    min: WorldPoint,
    max: WorldPoint,
    zoom: u8,
) -> impl Iterator<Item = TileCoord> {
    let width = matrix_width(zoom);
    let min_x = matrix_index(min.x.min(max.x), width);
    let max_x = matrix_index(min.x.max(max.x), width);
    let min_y = matrix_index(min.y.min(max.y), width);
    let max_y = matrix_index(min.y.max(max.y), width);
    (min_x..=max_x).flat_map(move |x| (min_y..=max_y).map(move |y| TileCoord { zoom, x, y }))
}

pub(crate) fn bounds(tile: TileCoord) -> TileBounds {
    let width = f64::from(matrix_width(tile.zoom));
    TileBounds {
        min_x: f64::from(tile.x) / width,
        min_y: f64::from(tile.y) / width,
        max_x: f64::from(tile.x.wrapping_add(1)) / width,
        max_y: f64::from(tile.y.wrapping_add(1)) / width,
    }
}

pub(crate) fn local_point(point: WorldPoint, tile: TileCoord) -> (i32, i32) {
    let width = f64::from(matrix_width(tile.zoom));
    let x = ((point.x * width - f64::from(tile.x)) * f64::from(EXTENT)).round();
    let y = ((point.y * width - f64::from(tile.y)) * f64::from(EXTENT)).round();
    (clamp_extent(x), clamp_extent(y))
}

fn matrix_width(zoom: u8) -> u32 {
    1u32 << u32::from(zoom)
}

fn matrix_index(value: f64, width: u32) -> u32 {
    let index = (value * f64::from(width)).floor();
    let index = if index.is_finite() && index >= 0.0 {
        index as u32
    } else {
        0
    };
    index.min(width.wrapping_sub(1))
}

fn clamp_extent(value: f64) -> i32 {
    if value <= 0.0 {
        0
    } else if value >= f64::from(EXTENT) {
        EXTENT
    } else {
        value as i32
    }
}
