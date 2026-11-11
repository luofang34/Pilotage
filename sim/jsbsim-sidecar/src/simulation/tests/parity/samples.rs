//! The frames that a controller reads must agree during a landing.

use pilotage_world::{MagneticEpoch, UniformTerrain, World};

use super::super::{message_id, vehicle};
use super::{
    Aircraft, Encoder, EngineKind, GROUND_M, ModelPaths, N51, Simulation, Start, assert_same_state,
    close, controls,
};
use crate::hil::{HIL_GPS, HIL_SENSOR};

fn f32_at(frame: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(
        frame[10 + at..14 + at].try_into().unwrap(),
    ))
}

fn i32_at(frame: &[u8], at: usize) -> i64 {
    i64::from(i32::from_le_bytes(
        frame[10 + at..14 + at].try_into().unwrap(),
    ))
}

fn assert_frames(at: &str, rust: &[Vec<u8>], cpp: &[Vec<u8>]) {
    assert_eq!(rust.len(), cpp.len(), "{at}: frame count");
    for (r, c) in rust.iter().zip(cpp) {
        assert_eq!(message_id(r), message_id(c));
        if message_id(r) != 0 {
            assert_eq!(&r[10..18], &c[10..18], "{at}: time");
        }
        if message_id(r) == HIL_SENSOR.0 {
            // Acceleration, rate, field, pressure, pitot, altitude, temperature.
            let limits = [
                0.02, 0.02, 0.02, 1e-4, 1e-4, 1e-4, 1e-4, 1e-4, 1e-4, 0.01, 0.001, 0.002, 1e-4,
            ];
            for (i, limit) in limits.into_iter().enumerate() {
                close(
                    at,
                    "HIL_SENSOR",
                    f32_at(r, 8 + 4 * i),
                    f32_at(c, 8 + 4 * i),
                    limit,
                );
            }
            assert_eq!(&r[70..74], &c[70..74], "{at}: sensor flags");
        }
        if message_id(r) == HIL_GPS.0 {
            for at in [8, 12, 16] {
                assert!(
                    (i32_at(r, at) - i32_at(c, at)).abs() <= 1,
                    "{at}: HIL_GPS at {at}"
                );
            }
            assert_eq!(&r[30..34], &c[30..34], "GNSS accuracy");
            for at in [26, 28, 30] {
                let value =
                    |frame: &[u8]| i16::from_le_bytes(frame[10 + at..12 + at].try_into().unwrap());
                assert!(
                    (i32::from(value(r)) - i32::from(value(c))).abs() <= 1,
                    "{at}: HIL_GPS velocity at {at}"
                );
            }
            let u16_at = |frame: &[u8], at: usize| {
                i32::from(u16::from_le_bytes(
                    frame[10 + at..12 + at].try_into().unwrap(),
                ))
            };
            assert!((u16_at(r, 24) - u16_at(c, 24)).abs() <= 1, "GNSS speed");
            let course = (u16_at(r, 32) - u16_at(c, 32) + 18_000).rem_euclid(36_000) - 18_000;
            assert!(course.abs() <= 5, "{at}: GNSS course");
            assert_eq!(&r[44..r.len() - 2], &c[44..c.len() - 2], "GNSS fix fields");
        }
    }
}

#[test]
fn landing_truth_and_sensor_frames_agree_at_all_step_rates_and_sites() {
    // Include the southern hemisphere, high latitude, the date line,
    // negative terrain height, high terrain, and a heading near zero.
    for (latitude_deg, longitude_deg, heading_deg, ground) in [
        (
            N51.latitude_deg,
            N51.longitude_deg,
            N51.heading_deg,
            GROUND_M,
        ),
        (-33.9, 179.9999, 359.99, -200.0),
        (78.0, -179.9999, 0.0, 3_000.0),
    ] {
        for step_ms in [1, 2, 4] {
            let mut world = World::new(
                N51,
                UniformTerrain {
                    elevation_m: ground,
                },
                MagneticEpoch::from_decimal_year(2026.8),
            );
            let paths = ModelPaths {
                root: option_env!("PILOTAGE_JSBSIM_ROOT").map(std::path::PathBuf::from),
                aircraft_dir: Aircraft::X500.bundled_dir(),
                model: "x500".to_owned(),
            };
            let start = Start {
                latitude_deg,
                longitude_deg,
                heading_deg,
            };
            let mut config = vehicle(Aircraft::X500, paths, start, None, step_ms);
            let mut rust = Simulation::new(&config, &mut world).unwrap();
            config.engine = EngineKind::Cpp;
            let mut cpp = Simulation::new(&config, &mut world).unwrap();
            let mut r_encoder = Encoder::default();
            let mut c_encoder = Encoder::default();
            let mut peak = 0.0f64;
            for step in 0..8_000 / step_ms {
                let elapsed_ms = step * step_ms;
                let command = match elapsed_ms {
                    0..250 => controls(1.0, true),
                    250..1_250 => controls(0.82, true),
                    1_250..3_000 => controls(0.64, true),
                    3_000..4_000 => controls(0.0, true),
                    _ => controls(1.0, false),
                };
                let r = rust
                    .step(Some(&command), &mut world, &mut r_encoder)
                    .unwrap();
                let c = cpp
                    .step(Some(&command), &mut world, &mut c_encoder)
                    .unwrap();
                let at = format!(
                    "{latitude_deg}°, {longitude_deg}°, {ground} m, {step_ms} ms, step {step}"
                );
                assert_same_state(&at, &rust.truth(), &cpp.truth());
                assert_frames(&at, &r, &c);
                peak = peak.max(rust.truth().altitude_m - ground);
            }
            assert!(peak > 0.5, "the vehicle must leave the ground: {peak}");
            for simulation in [&mut rust, &mut cpp] {
                let truth = simulation.truth();
                assert!((truth.altitude_m - ground - 0.12).abs() < 0.02);
                assert!(truth.velocity_ned.iter().all(|v| v.abs() < 0.01));
            }
        }
    }
}
