use std::sync::Arc;

use super::{CameraPose, Pinhole, Renderer, SKY_RGB};
use crate::geodesy::LocalFrame;
use crate::tiles::{TILE_SIZE, TileImage, TileKey, TileSource, locate};

/// Tiles whose pixels encode their own column and row: red is the column,
/// green the row, and blue the low byte of the tile column.
struct CoordinateTiles {
    requests: Vec<TileKey>,
}

impl TileSource for CoordinateTiles {
    fn tile(&mut self, key: TileKey) -> Option<Arc<TileImage>> {
        self.requests.push(key);
        let mut rgb = Vec::new();
        for py in 0..TILE_SIZE {
            for px in 0..TILE_SIZE {
                rgb.extend_from_slice(&[px as u8, py as u8, key.x as u8]);
            }
        }
        TileImage::from_rgb(rgb).map(Arc::new)
    }
}

const LAT: f64 = 40.3317;
const LON: f64 = -74.6745;

fn pose(height_m: f64, pitch_deg: f64) -> CameraPose {
    CameraPose {
        time_ns: 0,
        pos_ned_m: [0.0, 0.0, -height_m],
        yaw_rad: 0.0,
        pitch_rad: pitch_deg.to_radians(),
        lat_deg: LAT,
        lon_deg: LON,
    }
}

fn pixel(rgb: &[u8], width: u32, u: u32, v: u32) -> [u8; 3] {
    let i = ((v * width + u) * 3) as usize;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

#[test]
fn the_focal_length_follows_the_diagonal_field_of_view() {
    let camera = Pinhole::new(640, 360, 82.1, 19);
    let expected = 734.25 / 2.0 / 41.05_f64.to_radians().tan();
    assert!(
        (camera.focal_px - expected).abs() < 0.5,
        "{}",
        camera.focal_px
    );
}

#[test]
fn the_centre_of_a_nadir_frame_samples_the_tile_pixel_under_the_vehicle() {
    let camera = Pinhole::new(640, 360, 82.1, 19);
    let mut tiles = CoordinateTiles {
        requests: Vec::new(),
    };
    let frame = LocalFrame::anchored_at(LAT, LON);
    let rgb = Renderer::new(camera, frame, 0, &mut tiles).render(&pose(30.0, -90.0));
    assert_eq!(rgb.len(), 640 * 360 * 3);
    let (key, px, py) = locate(LAT, LON, 19);
    let [r, g, b] = pixel(&rgb, 640, 320, 180);
    assert_eq!(
        b, key.x as u8,
        "the centre samples the tile under the vehicle"
    );
    assert!((i32::from(r) - px as i32).abs() <= 1, "{r} vs {px}");
    assert!((i32::from(g) - py as i32).abs() <= 1, "{g} vs {py}");
    assert!(tiles.requests.iter().all(|key| key.z == 19));
}

#[test]
fn a_nadir_frame_right_edge_lies_east_of_the_vehicle() {
    // A nadir camera at yaw 0 has image right = east and image up = north.
    let camera = Pinhole::new(640, 360, 82.1, 19);
    let mut tiles = CoordinateTiles {
        requests: Vec::new(),
    };
    let frame = LocalFrame::anchored_at(LAT, LON);
    let rgb = Renderer::new(camera, frame, 0, &mut tiles).render(&pose(30.0, -90.0));
    let centre = pixel(&rgb, 640, 320, 180);
    let right = pixel(&rgb, 640, 400, 180);
    let up = pixel(&rgb, 640, 320, 140);
    // Within one tile the column grows east and the row grows south.
    let east = i32::from(right[0]) - i32::from(centre[0])
        + 256 * (i32::from(right[2]) - i32::from(centre[2]));
    assert!(east > 0, "{centre:?} -> {right:?}");
    assert!(
        up[1] < centre[1] || up[2] != centre[2],
        "{centre:?} -> {up:?}"
    );
}

#[test]
fn rays_above_the_horizon_render_sky() {
    let camera = Pinhole::new(64, 36, 82.1, 19);
    let mut tiles = CoordinateTiles {
        requests: Vec::new(),
    };
    let frame = LocalFrame::anchored_at(LAT, LON);
    let rgb = Renderer::new(camera, frame, 0, &mut tiles).render(&pose(50.0, 0.0));
    assert_eq!(pixel(&rgb, 64, 32, 0), SKY_RGB);
    assert_ne!(pixel(&rgb, 64, 32, 35), SKY_RGB);
}
