//! The ground JSBSim stands the vehicle on: a grid of the world's
//! terrain heights around the vehicle, answered between grid points by
//! bilinear interpolation.
//!
//! JSBSim asks for the ground under every gear contact on every step, so
//! the answer must be memory-only. The grid is sampled from the world
//! once, and again when the vehicle has flown far from its centre.

use pilotage_world::{TerrainSource, World, WorldError, metres_per_degree};

/// The terrain under one point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundAnswer {
    /// Metres above mean sea level (the simulator's ellipsoid).
    pub height_m: f64,
    /// Rise per metre northward.
    pub slope_north: f64,
    /// Rise per metre eastward.
    pub slope_east: f64,
}

/// A terrain model JSBSim can ask during a step.
pub trait GroundModel {
    /// The ground under a geodetic latitude and longitude, radians, or
    /// `None` when the model has no answer.
    fn height(&self, lat_rad: f64, lon_rad: f64) -> Option<GroundAnswer>;
}

/// How many grid cells the patch extends each side of its centre.
pub const HALF_CELLS: usize = 32;
/// The finest grid spacing, m: finer than this adds no terrain detail.
const MIN_SPACING_M: f64 = 5.0;
/// The coarsest grid spacing, m.
const MAX_SPACING_M: f64 = 30.0;
/// The spacing for a world whose terrain has no cells (uniform), m.
const UNIFORM_SPACING_M: f64 = 10.0;

/// A square grid of terrain heights centred on one point.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundPatch {
    centre_lat_deg: f64,
    centre_lon_deg: f64,
    spacing_m: f64,
    half: usize,
    /// Metres per degree of latitude and of longitude at the centre.
    per_degree: (f64, f64),
    /// Row-major from south-west, `(2 * half + 1)` squared.
    heights_m: Vec<f64>,
}

impl GroundPatch {
    /// Samples the world's terrain on a grid around a point. Where the
    /// terrain has no coverage, the site's declared elevation stands.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError`] when the terrain cannot be read.
    pub fn sample_blocking<T: TerrainSource>(
        world: &mut World<T>,
        lat_deg: f64,
        lon_deg: f64,
        half: usize,
    ) -> Result<Self, WorldError> {
        let half = half.max(1);
        let declared_m = world.site().elevation_m;
        let spacing_m = match world.elevation_blocking(lat_deg, lon_deg)? {
            Some(sample) if sample.resolution_m > 0.0 => {
                sample.resolution_m.clamp(MIN_SPACING_M, MAX_SPACING_M)
            }
            _ => UNIFORM_SPACING_M,
        };
        let per_degree = metres_per_degree(lat_deg);
        let side = 2 * half + 1;
        let mut heights_m = Vec::with_capacity(side * side);
        for row in 0..side {
            let north_m = (row as f64 - half as f64) * spacing_m;
            for column in 0..side {
                let east_m = (column as f64 - half as f64) * spacing_m;
                let height = world
                    .elevation_blocking(
                        lat_deg + north_m / per_degree.0,
                        lon_deg + east_m / per_degree.1,
                    )?
                    .map_or(declared_m, |sample| sample.elevation_m());
                heights_m.push(height);
            }
        }
        Ok(Self {
            centre_lat_deg: lat_deg,
            centre_lon_deg: lon_deg,
            spacing_m,
            half,
            per_degree,
            heights_m,
        })
    }

    /// The grid spacing, m.
    pub fn spacing_m(&self) -> f64 {
        self.spacing_m
    }

    /// Whether a point is far enough from the centre that the grid
    /// should be sampled again around it: past half the grid's reach.
    pub fn needs_recentre(&self, lat_deg: f64, lon_deg: f64) -> bool {
        let (north_m, east_m) = self.offset_m(lat_deg, lon_deg);
        let reach_m = self.spacing_m * self.half as f64 / 2.0;
        north_m.abs() > reach_m || east_m.abs() > reach_m
    }

    fn offset_m(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        (
            (lat_deg - self.centre_lat_deg) * self.per_degree.0,
            (lon_deg - self.centre_lon_deg) * self.per_degree.1,
        )
    }

    fn at(&self, row: usize, column: usize) -> f64 {
        let side = 2 * self.half + 1;
        self.heights_m
            .get(row * side + column)
            .copied()
            .unwrap_or(f64::NAN)
    }

    /// The ground at a point, degrees: bilinear between grid points, and
    /// the nearest edge's height beyond the grid.
    pub fn answer(&self, lat_deg: f64, lon_deg: f64) -> GroundAnswer {
        let (north_m, east_m) = self.offset_m(lat_deg, lon_deg);
        let last = (2 * self.half) as f64;
        let row_f = (north_m / self.spacing_m + self.half as f64).clamp(0.0, last);
        let column_f = (east_m / self.spacing_m + self.half as f64).clamp(0.0, last);
        let row = (row_f.floor() as usize).min(2 * self.half - 1);
        let column = (column_f.floor() as usize).min(2 * self.half - 1);
        let (fr, fc) = (row_f - row as f64, column_f - column as f64);
        let (h00, h01) = (self.at(row, column), self.at(row, column + 1));
        let (h10, h11) = (self.at(row + 1, column), self.at(row + 1, column + 1));
        let south = h00 + (h01 - h00) * fc;
        let north = h10 + (h11 - h10) * fc;
        GroundAnswer {
            height_m: south + (north - south) * fr,
            slope_north: ((h10 - h00) * (1.0 - fc) + (h11 - h01) * fc) / self.spacing_m,
            slope_east: ((h01 - h00) * (1.0 - fr) + (h11 - h10) * fr) / self.spacing_m,
        }
    }
}

impl GroundModel for GroundPatch {
    fn height(&self, lat_rad: f64, lon_rad: f64) -> Option<GroundAnswer> {
        let answer = self.answer(lat_rad.to_degrees(), lon_rad.to_degrees());
        answer.height_m.is_finite().then_some(answer)
    }
}

#[cfg(test)]
mod tests;
