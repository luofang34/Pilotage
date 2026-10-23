//! Web-mercator ground imagery for the simulated camera.
//!
//! A tile is a 256 by 256 RGB image at one zoom level. The camera asks a
//! [`TileSource`] for tiles; the source can fetch them over HTTP or make a
//! procedural texture.

pub mod http;
pub mod procedural;

use std::sync::Arc;

/// Width and height of one tile, pixels.
pub const TILE_SIZE: u32 = 256;
/// Ground metres per pixel at zoom 0 on the equator.
pub const EQUATOR_METRES_PER_PIXEL: f64 = 156_543.033_928;

/// One tile address: zoom level, column, and row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// Zoom level.
    pub z: u8,
    /// Column, from the antimeridian eastwards.
    pub x: u32,
    /// Row, from the north edge southwards.
    pub y: u32,
}

impl TileKey {
    /// The tile one zoom level up that contains this tile.
    #[must_use]
    pub fn parent(self) -> Option<Self> {
        (self.z > 0).then(|| Self {
            z: self.z - 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }
}

/// The RGB pixels of one tile, row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileImage {
    rgb: Vec<u8>,
}

impl TileImage {
    /// Wraps `rgb` when it holds exactly one tile.
    #[must_use]
    pub fn from_rgb(rgb: Vec<u8>) -> Option<Self> {
        let expected = (TILE_SIZE * TILE_SIZE * 3) as usize;
        (rgb.len() == expected).then_some(Self { rgb })
    }

    /// The pixel at column `px` and row `py`, clamped into the tile.
    #[must_use]
    pub fn pixel(&self, px: u32, py: u32) -> [u8; 3] {
        let px = px.min(TILE_SIZE - 1) as usize;
        let py = py.min(TILE_SIZE - 1) as usize;
        let index = (py * TILE_SIZE as usize + px) * 3;
        match self.rgb.get(index..index + 3) {
            Some([r, g, b]) => [*r, *g, *b],
            _ => [0, 0, 0],
        }
    }
}

/// A supplier of tiles. It can block; the camera calls it on its own
/// worker thread.
pub trait TileSource: Send {
    /// The tile at `key`, or `None` when the source has none.
    fn tile(&mut self, key: TileKey) -> Option<Arc<TileImage>>;
}

/// The global web-mercator pixel position of a point at zoom `z`.
#[must_use]
pub fn mercator_pixel(lat_deg: f64, lon_deg: f64, z: u8) -> [f64; 2] {
    let size = f64::from(TILE_SIZE) * f64::from(1u32 << u32::from(z.min(30)));
    let lat = lat_deg.clamp(-85.051_128, 85.051_128).to_radians();
    let x = (lon_deg + 180.0) / 360.0 * size;
    let y = (1.0 - lat.tan().asinh() / core::f64::consts::PI) / 2.0 * size;
    [x, y]
}

/// The tile and the pixel in it that contain a point at zoom `z`.
#[must_use]
pub fn locate(lat_deg: f64, lon_deg: f64, z: u8) -> (TileKey, u32, u32) {
    let [x, y] = mercator_pixel(lat_deg, lon_deg, z);
    let tile = f64::from(TILE_SIZE);
    let max = f64::from(1u32 << u32::from(z.min(30))) - 1.0;
    let tile_x = (x / tile).floor().clamp(0.0, max);
    let tile_y = (y / tile).floor().clamp(0.0, max);
    let px = (x - tile_x * tile).floor().clamp(0.0, tile - 1.0);
    let py = (y - tile_y * tile).floor().clamp(0.0, tile - 1.0);
    (
        TileKey {
            z,
            x: tile_x as u32,
            y: tile_y as u32,
        },
        px as u32,
        py as u32,
    )
}

/// The zoom level whose pixels are no larger than `metres_per_pixel` at
/// `lat_deg`, capped at `max_zoom`.
#[must_use]
pub fn zoom_for_resolution(metres_per_pixel: f64, lat_deg: f64, max_zoom: u8) -> u8 {
    if !metres_per_pixel.is_finite() || metres_per_pixel <= 0.0 {
        return max_zoom;
    }
    let zoom = (EQUATOR_METRES_PER_PIXEL * lat_deg.to_radians().cos() / metres_per_pixel)
        .log2()
        .ceil();
    zoom.clamp(0.0, f64::from(max_zoom)) as u8
}

#[cfg(test)]
mod tests;
