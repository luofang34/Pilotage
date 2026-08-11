//! Web Mercator addressing and Terrarium pixel decoding.

use std::f64::consts::PI;
use std::io::Cursor;
use std::path::Path;

use png::{BitDepth, ColorType, Decoder};

use crate::TerrainQueryError;

pub(crate) const WEB_MERCATOR_MAX_LAT_DEG: f64 = 85.051_128_779_806_6;
const TERRARIUM_OFFSET_M: f64 = 32_768.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileAddress {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct PixelAddress {
    pub x: u32,
    pub y: u32,
}

pub(crate) struct DecodedTile {
    width: u32,
    pixels: Vec<u8>,
}

impl DecodedTile {
    pub(crate) fn decode(
        address: TileAddress,
        png_bytes: &[u8],
        expected_size: u32,
        archive_path: &Path,
    ) -> Result<Self, TerrainQueryError> {
        let decoder = Decoder::new(Cursor::new(png_bytes));
        let mut reader = decoder
            .read_info()
            .map_err(|source| decode_error(archive_path, address, source))?;
        let buffer_size = reader
            .output_buffer_size()
            .ok_or(TerrainQueryError::TileTooLarge {
                path: archive_path.to_path_buf(),
                zoom: address.zoom,
                x: address.x,
                y: address.y,
            })?;
        let mut pixels = vec![0; buffer_size];
        let info = reader
            .next_frame(&mut pixels)
            .map_err(|source| decode_error(archive_path, address, source))?;
        if info.width != expected_size
            || info.height != expected_size
            || info.color_type != ColorType::Rgb
            || info.bit_depth != BitDepth::Eight
        {
            return Err(TerrainQueryError::UnsupportedTile {
                path: archive_path.to_path_buf(),
                zoom: address.zoom,
                x: address.x,
                y: address.y,
                layout: format!(
                    "{}x{} {:?} {:?}",
                    info.width, info.height, info.color_type, info.bit_depth
                ),
            });
        }
        pixels.truncate(info.buffer_size());
        Ok(Self {
            width: info.width,
            pixels,
        })
    }

    pub(crate) fn elevation_m(&self, pixel: PixelAddress) -> Option<f64> {
        let index = (u64::from(pixel.y) * u64::from(self.width) + u64::from(pixel.x)) * 3;
        let index = usize::try_from(index).ok()?;
        let channels = self.pixels.get(index..index.checked_add(3)?)?;
        Some(
            f64::from(channels[0]) * 256.0
                + f64::from(channels[1])
                + f64::from(channels[2]) / 256.0
                - TERRARIUM_OFFSET_M,
        )
    }
}

pub(crate) fn address_for_position(
    zoom: u8,
    latitude_deg: f64,
    longitude_deg: f64,
    tile_size: u32,
) -> (TileAddress, PixelAddress) {
    let side = 1u32 << u32::from(zoom);
    let world_x = ((longitude_deg + 180.0) / 360.0).clamp(0.0, 1.0);
    let latitude_rad = latitude_deg.to_radians();
    let world_y = (1.0 - libm::asinh(libm::tan(latitude_rad)) / PI) / 2.0;
    let (x, pixel_x) = axis_address(world_x, side, tile_size);
    let (y, pixel_y) = axis_address(world_y, side, tile_size);
    (
        TileAddress { zoom, x, y },
        PixelAddress {
            x: pixel_x,
            y: pixel_y,
        },
    )
}

fn axis_address(world: f64, side: u32, tile_size: u32) -> (u32, u32) {
    let scaled = world.clamp(0.0, 1.0) * f64::from(side);
    let tile = scaled.floor().clamp(0.0, f64::from(side - 1)) as u32;
    let fraction = (scaled - f64::from(tile)).clamp(0.0, 1.0);
    let pixel = (fraction * f64::from(tile_size))
        .floor()
        .clamp(0.0, f64::from(tile_size - 1)) as u32;
    (tile, pixel)
}

fn decode_error(
    archive_path: &Path,
    address: TileAddress,
    source: png::DecodingError,
) -> TerrainQueryError {
    TerrainQueryError::TileDecode {
        path: archive_path.to_path_buf(),
        zoom: address.zoom,
        x: address.x,
        y: address.y,
        source,
    }
}
