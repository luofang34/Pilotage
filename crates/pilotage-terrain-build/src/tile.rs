//! Web Mercator tile selection and Terrarium PNG encoding.

use std::f64::consts::PI;

use png::{BitDepth, ColorType, Compression, Encoder};

use crate::TerrainBuildError;
use crate::config::TerrainBuildConfig;
use crate::sampler::TerrainSampler;

pub(crate) const TILE_SIZE: u32 = 256;
const TILE_LIMIT: u64 = 131_072;
const TERRARIUM_OFFSET_M: f64 = 32_768.0;
const TERRARIUM_MAX_CODE: f64 = 16_777_215.0;

pub(crate) struct RasterTile {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
    pub png: Vec<u8>,
}

#[derive(Clone, Copy)]
struct TileAddress {
    zoom: u8,
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct IndexRange {
    start: u32,
    end: u32,
}

pub(crate) fn rasterize_tiles(
    sampler: &TerrainSampler<'_>,
    config: TerrainBuildConfig,
) -> Result<Vec<RasterTile>, TerrainBuildError> {
    let count = selected_tile_count(config)?;
    let capacity = usize::try_from(count).map_err(|_| TerrainBuildError::TooManyTiles {
        count,
        limit: TILE_LIMIT,
    })?;
    let mut tiles = Vec::with_capacity(capacity);
    for zoom in config.min_zoom..=config.max_zoom {
        let (xs, ys) = tile_ranges(config, zoom);
        for y in ys.start..=ys.end {
            for x in xs.start..=xs.end {
                let address = TileAddress { zoom, x, y };
                tiles.push(rasterize_tile(sampler, address)?);
            }
        }
    }
    Ok(tiles)
}

fn selected_tile_count(config: TerrainBuildConfig) -> Result<u64, TerrainBuildError> {
    let mut count = 0u64;
    for zoom in config.min_zoom..=config.max_zoom {
        let (xs, ys) = tile_ranges(config, zoom);
        let width = u64::from(xs.end - xs.start + 1);
        let height = u64::from(ys.end - ys.start + 1);
        count = count.saturating_add(width.saturating_mul(height));
    }
    if count > TILE_LIMIT {
        return Err(TerrainBuildError::TooManyTiles {
            count,
            limit: TILE_LIMIT,
        });
    }
    Ok(count)
}

fn tile_ranges(config: TerrainBuildConfig, zoom: u8) -> (IndexRange, IndexRange) {
    let side = 1u32 << u32::from(zoom);
    let west = longitude_world_x(config.region.min_lon_deg);
    let east = longitude_world_x(config.region.max_lon_deg);
    let north = latitude_world_y(config.region.max_lat_deg);
    let south = latitude_world_y(config.region.min_lat_deg);
    (
        world_index_range(west, east, side),
        world_index_range(north, south, side),
    )
}

fn world_index_range(min: f64, max: f64, side: u32) -> IndexRange {
    let side_f64 = f64::from(side);
    let last = side.saturating_sub(1);
    let start = (min * side_f64).floor().clamp(0.0, f64::from(last)) as u32;
    let end = ((max * side_f64).ceil() - 1.0).clamp(0.0, f64::from(last)) as u32;
    IndexRange { start, end }
}

fn rasterize_tile(
    sampler: &TerrainSampler<'_>,
    address: TileAddress,
) -> Result<RasterTile, TerrainBuildError> {
    let pixels = render_pixels(sampler, address)?;
    let png = encode_png(&pixels, address)?;
    Ok(RasterTile {
        zoom: address.zoom,
        x: address.x,
        y: address.y,
        png,
    })
}

fn render_pixels(
    sampler: &TerrainSampler<'_>,
    address: TileAddress,
) -> Result<Vec<u8>, TerrainBuildError> {
    let mut pixels = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 3) as usize);
    for pixel_y in 0..TILE_SIZE {
        for pixel_x in 0..TILE_SIZE {
            let (lat, lon) = pixel_center(address, pixel_x, pixel_y);
            let elevation =
                sampler
                    .sample_msl(lat, lon)?
                    .ok_or(TerrainBuildError::MissingElevation {
                        zoom: address.zoom,
                        x: address.x,
                        y: address.y,
                        pixel_x: pixel_x as u16,
                        pixel_y: pixel_y as u16,
                    })?;
            pixels.extend_from_slice(&terrarium_rgb(elevation)?);
        }
    }
    Ok(pixels)
}

fn pixel_center(address: TileAddress, pixel_x: u32, pixel_y: u32) -> (f64, f64) {
    let side = f64::from(1u32 << u32::from(address.zoom));
    let x = (f64::from(address.x) + (f64::from(pixel_x) + 0.5) / f64::from(TILE_SIZE)) / side;
    let y = (f64::from(address.y) + (f64::from(pixel_y) + 0.5) / f64::from(TILE_SIZE)) / side;
    let lon = x * 360.0 - 180.0;
    let lat = libm::atan(libm::sinh(PI * (1.0 - 2.0 * y))) * 180.0 / PI;
    (lat, lon)
}

fn longitude_world_x(longitude_deg: f64) -> f64 {
    (longitude_deg + 180.0) / 360.0
}

fn latitude_world_y(latitude_deg: f64) -> f64 {
    let latitude_rad = latitude_deg * PI / 180.0;
    (1.0 - libm::asinh(libm::tan(latitude_rad)) / PI) / 2.0
}

pub(crate) fn terrarium_rgb(elevation_m: f64) -> Result<[u8; 3], TerrainBuildError> {
    let code = ((elevation_m + TERRARIUM_OFFSET_M) * 256.0).round();
    if !(code.is_finite() && (0.0..=TERRARIUM_MAX_CODE).contains(&code)) {
        return Err(TerrainBuildError::ElevationOutsideTerrarium { elevation_m });
    }
    let code = code as u32;
    Ok([
        ((code >> 16) & 0xff) as u8,
        ((code >> 8) & 0xff) as u8,
        (code & 0xff) as u8,
    ])
}

fn encode_png(pixels: &[u8], address: TileAddress) -> Result<Vec<u8>, TerrainBuildError> {
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, TILE_SIZE, TILE_SIZE);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::High);
    let mut writer = encoder
        .write_header()
        .map_err(|source| png_error(address, source))?;
    writer
        .write_image_data(pixels)
        .map_err(|source| png_error(address, source))?;
    writer
        .finish()
        .map_err(|source| png_error(address, source))?;
    Ok(bytes)
}

fn png_error(address: TileAddress, source: png::EncodingError) -> TerrainBuildError {
    TerrainBuildError::PngEncoding {
        zoom: address.zoom,
        x: address.x,
        y: address.y,
        source,
    }
}
