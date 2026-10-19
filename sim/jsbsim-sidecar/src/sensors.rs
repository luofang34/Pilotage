//! Sensor readings synthesized from the true state and the world model:
//! an IMU, a barometer, a magnetometer from the world's WMM field, an
//! airspeed sensor, and a GNSS receiver, each with seeded noise.
//!
//! Every simulated vehicle of a world reads its magnetometer from the
//! same field model at the same epoch, so every flight controller sees
//! the same declination.

mod noise;

pub use noise::Noise;

use pilotage_world::{Ned, PositionError, TerrainSource, World, WorldError, local_plane};

use crate::truth::Truth;

/// One standard deviation of each sensor's white noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseLevels {
    /// Accelerometer, m/s².
    pub accel_mps2: f64,
    /// Gyroscope, rad/s.
    pub gyro_rad_s: f64,
    /// Magnetometer, gauss.
    pub mag_gauss: f64,
    /// Barometer, Pa.
    pub baro_pa: f64,
    /// Differential pressure, Pa.
    pub diff_pressure_pa: f64,
    /// GNSS horizontal position, m.
    pub gnss_horizontal_m: f64,
    /// GNSS vertical position, m.
    pub gnss_vertical_m: f64,
    /// GNSS velocity, m/s.
    pub gnss_velocity_mps: f64,
}

/// Noise of the order of a small-UAV MEMS IMU and a u-blox class GNSS.
pub const DEFAULT_NOISE: NoiseLevels = NoiseLevels {
    accel_mps2: 0.02,
    gyro_rad_s: 0.002,
    mag_gauss: 0.001,
    baro_pa: 2.0,
    diff_pressure_pa: 0.5,
    gnss_horizontal_m: 0.15,
    gnss_vertical_m: 0.25,
    gnss_velocity_mps: 0.05,
};

/// One IMU, barometer, magnetometer, and airspeed reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InertialSample {
    /// Simulation time, µs.
    pub time_us: u64,
    /// Specific force, body FRD, m/s².
    pub accel_frd: [f64; 3],
    /// Angular rate, body FRD, rad/s.
    pub gyro_frd: [f64; 3],
    /// Magnetic field, body FRD, gauss.
    pub mag_frd_gauss: [f64; 3],
    /// Static pressure, hPa.
    pub pressure_hpa: f64,
    /// Differential (pitot) pressure, hPa.
    pub diff_pressure_hpa: f64,
    /// Air temperature, °C.
    pub temperature_c: f64,
}

/// One GNSS fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GnssSample {
    /// Simulation time, µs.
    pub time_us: u64,
    /// Latitude, degrees.
    pub latitude_deg: f64,
    /// Longitude, degrees.
    pub longitude_deg: f64,
    /// Altitude above mean sea level, m.
    pub altitude_m: f64,
    /// Velocity, NED, m/s.
    pub velocity_ned: [f64; 3],
    /// Horizontal accuracy, m.
    pub horizontal_accuracy_m: f64,
    /// Vertical accuracy, m.
    pub vertical_accuracy_m: f64,
}

/// The sensors of one vehicle.
#[derive(Debug, Clone)]
pub struct Sensors {
    noise: Noise,
    levels: NoiseLevels,
}

impl Sensors {
    /// Sensors with `levels` of noise from `seed`.
    pub fn new(seed: u64, levels: NoiseLevels) -> Self {
        Self {
            noise: Noise::new(seed),
            levels,
        }
    }

    /// The IMU, barometer, magnetometer, and airspeed readings at `truth`.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError`] when the world cannot give the magnetic
    /// field or the air at the vehicle's position.
    pub fn inertial<T: TerrainSource>(
        &mut self,
        truth: &Truth,
        world: &World<T>,
    ) -> Result<InertialSample, WorldError> {
        let field =
            world.magnetic_field(truth.latitude_deg, truth.longitude_deg, truth.altitude_m)?;
        let field_ned_gauss = [field.north_nt, field.east_nt, field.down_nt].map(|nt| nt * 1.0e-5);
        let mag_frd = truth.attitude.ned_to_body(field_ned_gauss);
        let levels = self.levels;
        let add = |value: [f64; 3], noise: [f64; 3]| [0, 1, 2].map(|i| value[i] + noise[i]);
        let accel_noise = self.noise.gaussian3(levels.accel_mps2);
        let gyro_noise = self.noise.gaussian3(levels.gyro_rad_s);
        let mag_noise = self.noise.gaussian3(levels.mag_gauss);
        // The barometer reads the world's air, the same model every
        // simulated vehicle and flight controller of the world shares.
        let air = world.atmosphere(truth.latitude_deg, truth.longitude_deg, truth.altitude_m)?;
        let pressure_pa = air.pressure_pa + self.noise.gaussian(levels.baro_pa);
        let diff_pa =
            (truth.dynamic_pressure_pa + self.noise.gaussian(levels.diff_pressure_pa)).max(0.0);
        Ok(InertialSample {
            time_us: time_us(truth.time_s),
            accel_frd: add(truth.specific_force_frd, accel_noise),
            gyro_frd: add(truth.rates_frd, gyro_noise),
            mag_frd_gauss: add(mag_frd, mag_noise),
            pressure_hpa: pressure_pa / 100.0,
            diff_pressure_hpa: diff_pa / 100.0,
            temperature_c: air.temperature_c(),
        })
    }

    /// A GNSS fix at `truth`: the truth moved by the receiver's noise in
    /// the world model's local frame there.
    ///
    /// # Errors
    ///
    /// Returns [`PositionError`] for a truth position not well formed.
    pub fn gnss(&mut self, truth: &Truth) -> Result<GnssSample, PositionError> {
        let levels = self.levels;
        let plane = local_plane(truth.latitude_deg, truth.longitude_deg, truth.altitude_m)?;
        let fix = plane.to_geodetic(Ned {
            north_m: self.noise.gaussian(levels.gnss_horizontal_m),
            east_m: self.noise.gaussian(levels.gnss_horizontal_m),
            down_m: -self.noise.gaussian(levels.gnss_vertical_m),
        });
        let velocity_noise = self.noise.gaussian3(levels.gnss_velocity_mps);
        Ok(GnssSample {
            time_us: time_us(truth.time_s),
            latitude_deg: fix.latitude_deg,
            longitude_deg: fix.longitude_deg,
            // The world's MSL heights are its ellipsoidal heights.
            altitude_m: fix.ellipsoidal_height_m,
            velocity_ned: [0, 1, 2].map(|i| truth.velocity_ned[i] + velocity_noise[i]),
            horizontal_accuracy_m: 3.0 * levels.gnss_horizontal_m,
            vertical_accuracy_m: 3.0 * levels.gnss_vertical_m,
        })
    }
}

/// Simulation time in whole microseconds, the unit of PX4's clock.
pub fn time_us(time_s: f64) -> u64 {
    (time_s * 1.0e6).round().max(0.0) as u64
}

#[cfg(test)]
mod tests;
