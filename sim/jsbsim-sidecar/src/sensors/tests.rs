//! Sensor synthesis: the IMU reads specific force and rate, the
//! magnetometer reads the world's field in body axes, and noise is
//! seeded and of the stated size.

#![allow(clippy::unwrap_used)]

use pilotage_world::{MagneticEpoch, N51, UniformTerrain, World};
use world_atmosphere::{Altitude, Isa};

use super::{DEFAULT_NOISE, Noise, NoiseLevels, Sensors, time_us};
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
fn the_imu_reads_specific_force_and_rate() {
    let mut sensors = Sensors::new(1, QUIET);
    let sample = sensors.inertial(&resting(0.0), &world()).unwrap();
    assert_eq!(sample.accel_frd, [0.0, 0.0, -9.806_65]);
    assert_eq!(sample.gyro_frd, [0.01, -0.02, 0.03]);
    assert_eq!(sample.time_us, 1_500_000);
}

/// Without noise, every reading derived from the world is the world
/// model's own value: the barometer the shared atmosphere, the
/// magnetometer the shared field, the GNSS the true position.
#[test]
fn the_sensors_read_the_world_model_at_every_position() {
    let world = world();
    let mut sensors = Sensors::new(1, QUIET);
    for (latitude_deg, longitude_deg, altitude_m) in [
        (N51.latitude_deg, N51.longitude_deg, 56.4),
        (47.397_742, 8.545_594, 488.0),
        (-33.9, 151.2, 3_000.0),
    ] {
        let mut truth = resting(0.0);
        truth.latitude_deg = latitude_deg;
        truth.longitude_deg = longitude_deg;
        truth.altitude_m = altitude_m;
        let sample = sensors.inertial(&truth, &world).unwrap();
        let air = world
            .atmosphere(latitude_deg, longitude_deg, altitude_m)
            .unwrap();
        assert_eq!(sample.pressure_hpa, air.pressure_pa / 100.0);
        assert_eq!(sample.temperature_c, air.temperature_c());
        // The pressure altitude is where the standard atmosphere has the
        // air's pressure.
        let standard = Isa
            .at(Altitude::Geopotential(sample.pressure_altitude_m))
            .unwrap();
        assert!((standard.pressure_pa - air.pressure_pa).abs() < 1e-6);
        let field = world
            .magnetic_field(latitude_deg, longitude_deg, altitude_m)
            .unwrap();
        // Level and facing north, body axes are north-east-down.
        let ned = [field.north_nt, field.east_nt, field.down_nt].map(|nt| nt * 1.0e-5);
        for (got, want) in sample.mag_frd_gauss.iter().zip(ned) {
            assert!((got - want).abs() < 1e-12, "{got} vs {want}");
        }
        let fix = sensors.gnss(&truth).unwrap();
        assert!((fix.latitude_deg - latitude_deg).abs() < 1e-12);
        assert!((fix.longitude_deg - longitude_deg).abs() < 1e-12);
        assert!((fix.altitude_m - altitude_m).abs() < 1e-6);
    }
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
    let truth = resting(0.0);
    let fix = sensors.gnss(&truth).unwrap();
    let plane =
        pilotage_world::local_plane(truth.latitude_deg, truth.longitude_deg, truth.altitude_m)
            .unwrap();
    let offset = plane
        .to_ned(pilotage_world::world_position(
            fix.latitude_deg,
            fix.longitude_deg,
            fix.altitude_m,
        ))
        .unwrap();
    // Within five sigma of 0.15 m horizontally and 0.25 m vertically.
    assert!(offset.north_m.hypot(offset.east_m) < 1.0, "{offset:?}");
    assert!(offset.down_m.abs() < 1.25, "{offset:?}");
    assert_eq!(time_us(0.004), 4_000);
}
