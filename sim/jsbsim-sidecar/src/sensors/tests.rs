//! Sensor synthesis: the IMU reads specific force and rate, the
//! magnetometer reads the world's field in body axes, and noise is
//! seeded and of the stated size.

#![allow(clippy::unwrap_used)]

use pilotage_world::{MagneticEpoch, N51, UniformTerrain, World};

use super::{DEFAULT_NOISE, Noise, NoiseLevels, Sensors, pressure_altitude_m, time_us};
use crate::rotation::Euler;
use crate::truth::Truth;

const QUIET: NoiseLevels = NoiseLevels {
    accel_mps2: 0.0,
    gyro_rad_s: 0.0,
    mag_gauss: 0.0,
    baro_pa: 0.0,
    diff_pressure_pa: 0.0,
    gnss_horizontal_m: 0.0,
    gnss_vertical_m: 0.0,
    gnss_velocity_mps: 0.0,
};

fn resting(yaw_deg: f64) -> Truth {
    Truth {
        time_s: 1.5,
        latitude_deg: N51.latitude_deg,
        longitude_deg: N51.longitude_deg,
        altitude_m: 56.4,
        velocity_ned: [0.0; 3],
        attitude: Euler {
            roll: 0.0,
            pitch: 0.0,
            yaw: yaw_deg.to_radians(),
        },
        rates_frd: [0.01, -0.02, 0.03],
        specific_force_frd: [0.0, 0.0, -9.806_65],
        pressure_pa: 100_650.0,
        temperature_c: 14.6,
        dynamic_pressure_pa: 0.0,
        true_airspeed_mps: 0.0,
        calibrated_airspeed_mps: 0.0,
    }
}

fn world() -> World<UniformTerrain> {
    World::new(
        N51,
        UniformTerrain { elevation_m: 56.3 },
        MagneticEpoch::from_decimal_year(2026.8),
    )
}

/// A level magnetometer's heading, corrected by the declination, is the
/// vehicle's true heading: what an FC's heading should match.
#[test]
fn the_magnetometer_heading_plus_declination_is_the_true_heading() {
    let world = world();
    let declination = world
        .magnetic_field(N51.latitude_deg, N51.longitude_deg, 56.4)
        .unwrap()
        .declination_deg;
    let mut sensors = Sensors::new(1, QUIET);
    for yaw in [0.0, 29.5, 135.0, 270.0] {
        let sample = sensors.inertial(&resting(yaw), &world).unwrap();
        let [mx, my, mz] = sample.mag_frd_gauss;
        let magnetic_heading = (-my).atan2(mx).to_degrees();
        let true_heading = (magnetic_heading + declination).rem_euclid(360.0);
        let error = (true_heading - yaw + 540.0).rem_euclid(360.0) - 180.0;
        assert!(error.abs() < 1e-6, "yaw {yaw}: {true_heading}");
        // The field dips down at N51: positive body z on a level vehicle.
        assert!(mz > 0.4 && mz < 0.6, "{mz}");
    }
}

#[test]
fn the_imu_reads_specific_force_and_rate_and_the_barometer_its_pressure() {
    let mut sensors = Sensors::new(1, QUIET);
    let sample = sensors.inertial(&resting(0.0), &world()).unwrap();
    assert_eq!(sample.accel_frd, [0.0, 0.0, -9.806_65]);
    assert_eq!(sample.gyro_frd, [0.01, -0.02, 0.03]);
    assert!((sample.pressure_hpa - 1006.5).abs() < 1e-9);
    assert!((sample.pressure_altitude_m - pressure_altitude_m(100_650.0)).abs() < 1e-9);
    assert_eq!(sample.time_us, 1_500_000);
    // The ISA: 1013.25 hPa is 0 m and 1006.5 hPa about 56 m.
    assert!(pressure_altitude_m(101_325.0).abs() < 1e-6);
    assert!((pressure_altitude_m(100_650.0) - 56.4).abs() < 0.5);
}

#[test]
fn noise_is_seeded_and_of_the_stated_size() {
    let mut a = Noise::new(42);
    let mut b = Noise::new(42);
    let samples: Vec<f64> = (0..20_000).map(|_| a.gaussian(0.5)).collect();
    let again: Vec<f64> = (0..20_000).map(|_| b.gaussian(0.5)).collect();
    assert_eq!(samples, again);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let sd =
        (samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
    assert!(mean.abs() < 0.02, "{mean}");
    assert!((sd - 0.5).abs() < 0.02, "{sd}");
    let mut sensors = Sensors::new(7, DEFAULT_NOISE);
    let fix = sensors.gnss(&resting(0.0));
    let (per_lat, _) = pilotage_world::metres_per_degree(N51.latitude_deg);
    assert!(((fix.latitude_deg - N51.latitude_deg) * per_lat).abs() < 1.0);
    assert_eq!(time_us(0.004), 4_000);
}
