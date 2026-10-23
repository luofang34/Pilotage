//! The pinhole camera: each pixel ray meets the flat ground at the launch
//! altitude, and the ground point is coloured from the tiles.

use std::collections::HashMap;
use std::sync::Arc;

use crate::geodesy::{LocalFrame, quaternion_from_euler, rotate};
use crate::tiles::procedural::ground_rgb;
use crate::tiles::{TILE_SIZE, TileImage, TileKey, TileSource, locate, zoom_for_resolution};

/// The colour of a pixel whose ray does not meet the ground.
pub const SKY_RGB: [u8; 3] = [150, 190, 230];
/// Ground beyond this range is drawn as sky haze, metres.
const MAX_RANGE_M: f64 = 4_000.0;
/// The lowest zoom level the camera asks for.
const MIN_ZOOM: u8 = 10;
/// How many coarser levels stand in for a missing tile.
const PARENT_LEVELS: u8 = 4;

/// Where the camera is and where it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Simulation time of the pose, nanoseconds.
    pub time_ns: u64,
    /// Camera position north, east, down from the launch point, metres.
    pub pos_ned_m: [f64; 3],
    /// Camera heading, radians from true north.
    pub yaw_rad: f64,
    /// Camera pitch, radians; negative looks down.
    pub pitch_rad: f64,
    /// Vehicle latitude, degrees.
    pub lat_deg: f64,
    /// Vehicle longitude, degrees.
    pub lon_deg: f64,
}

/// The fixed geometry of one camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pinhole {
    /// Image width, pixels.
    pub width: u32,
    /// Image height, pixels.
    pub height: u32,
    /// Focal length, pixels.
    pub focal_px: f64,
    /// The highest tile zoom level.
    pub max_zoom: u8,
}

impl Pinhole {
    /// A camera of `width` by `height` pixels with a diagonal field of
    /// view of `diagonal_fov_deg`.
    #[must_use]
    pub fn new(width: u32, height: u32, diagonal_fov_deg: f64, max_zoom: u8) -> Self {
        let diagonal = f64::from(width).hypot(f64::from(height));
        let half_fov = (diagonal_fov_deg.clamp(1.0, 170.0) / 2.0).to_radians();
        Self {
            width,
            height,
            focal_px: diagonal / 2.0 / half_fov.tan(),
            max_zoom,
        }
    }
}

/// Renders one RGB frame.
pub struct Renderer<'a> {
    camera: Pinhole,
    frame: LocalFrame,
    seed: u64,
    tiles: &'a mut dyn TileSource,
    resolved: HashMap<TileKey, Option<Arc<TileImage>>>,
}

impl<'a> Renderer<'a> {
    /// A renderer over `tiles` for the ground around `frame`.
    pub fn new(
        camera: Pinhole,
        frame: LocalFrame,
        seed: u64,
        tiles: &'a mut dyn TileSource,
    ) -> Self {
        Self {
            camera,
            frame,
            seed,
            tiles,
            resolved: HashMap::new(),
        }
    }

    /// The RGB pixels, row-major, of the view from `pose`.
    pub fn render(&mut self, pose: &CameraPose) -> Vec<u8> {
        let Pinhole {
            width,
            height,
            focal_px,
            ..
        } = self.camera;
        let orientation = quaternion_from_euler(0.0, pose.pitch_rad, pose.yaw_rad);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        for v in 0..height {
            for u in 0..width {
                let ray = [focal_px, f64::from(u) + 0.5 - cx, f64::from(v) + 0.5 - cy];
                let norm = (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
                let ray = rotate(orientation, ray.map(|component| component / norm));
                rgb.extend_from_slice(&self.ground_colour(pose, ray));
            }
        }
        rgb
    }

    fn ground_colour(&mut self, pose: &CameraPose, ray: [f64; 3]) -> [u8; 3] {
        let height_m = (-pose.pos_ned_m[2]).max(0.0);
        if ray[2] <= 1e-6 {
            return SKY_RGB;
        }
        let range = height_m / ray[2];
        if range > MAX_RANGE_M {
            return SKY_RGB;
        }
        let north = pose.pos_ned_m[0] + range * ray[0];
        let east = pose.pos_ned_m[1] + range * ray[1];
        let [lat, lon] = self.frame.lat_lon(north, east);
        // The ground footprint of one pixel grows with range and with a
        // low grazing angle.
        let metres_per_pixel = range / (self.camera.focal_px * ray[2]);
        let zoom = zoom_for_resolution(metres_per_pixel, lat, self.camera.max_zoom)
            .max(MIN_ZOOM.min(self.camera.max_zoom));
        self.sample(lat, lon, zoom)
    }

    /// The tile colour at a ground point, from the best tile available.
    pub fn sample(&mut self, lat_deg: f64, lon_deg: f64, zoom: u8) -> [u8; 3] {
        let lowest = zoom.saturating_sub(PARENT_LEVELS);
        for level in (lowest..=zoom).rev() {
            let (key, px, py) = locate(lat_deg, lon_deg, level);
            if let Some(tile) = self.resolve(key) {
                return tile.pixel(px.min(TILE_SIZE - 1), py.min(TILE_SIZE - 1));
            }
        }
        // No imagery here: the procedural texture keeps the frame usable.
        let [x, y] = crate::tiles::mercator_pixel(lat_deg, lon_deg, 0);
        let metres_per_unit = 40_075_016.686 / f64::from(TILE_SIZE);
        ground_rgb(x * metres_per_unit, y * metres_per_unit, self.seed)
    }

    fn resolve(&mut self, key: TileKey) -> Option<Arc<TileImage>> {
        if let Some(found) = self.resolved.get(&key) {
            return found.clone();
        }
        let found = self.tiles.tile(key);
        self.resolved.insert(key, found.clone());
        found
    }
}

#[cfg(test)]
mod tests;
