//! A whole vehicle over the real JSBSim library: it stands on the
//! world's ground at its start heading, climbs on PX4's motor commands,
//! and sends PX4 its frames at their rates.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use pilotage_world::{MagneticEpoch, N51, UniformTerrain, World};

use super::{Simulation, Start};
use crate::aircraft::Aircraft;
use crate::engine::ModelPaths;
use crate::hil::{
    ActuatorControls, Encoder, HIL_GPS, HIL_SENSOR, HIL_STATE_QUATERNION, MODE_ARMED,
};
use crate::sensors::DEFAULT_NOISE;

const GROUND_M: f64 = 120.0;

fn world() -> World<UniformTerrain> {
    World::new(
        N51,
        UniformTerrain {
            elevation_m: GROUND_M,
        },
        MagneticEpoch::from_decimal_year(2026.8),
    )
}

fn x500(world: &mut World<UniformTerrain>) -> Simulation {
    let paths = ModelPaths {
        root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
        aircraft_dir: Aircraft::X500.bundled_dir(),
        model: "x500".to_owned(),
    };
    let start = Start {
        latitude_deg: N51.latitude_deg,
        longitude_deg: N51.longitude_deg,
        heading_deg: N51.heading_deg,
    };
    Simulation::new(Aircraft::X500, &paths, world, start, 3, DEFAULT_NOISE, None).unwrap()
}

fn controls(command: f32, armed: bool) -> ActuatorControls {
    let mut controls = [0.0; 16];
    controls[..4].fill(command);
    ActuatorControls {
        time_us: 0,
        controls,
        mode: if armed { MODE_ARMED } else { 0 },
        flags: 1,
    }
}

fn message_id(frame: &[u8]) -> u32 {
    u32::from(frame[7]) | u32::from(frame[8]) << 8 | u32::from(frame[9]) << 16
}

#[test]
fn the_x500_stands_on_the_ground_at_its_start_heading() {
    let mut world = world();
    let mut simulation = x500(&mut world);
    let mut encoder = Encoder::default();
    for _ in 0..500 {
        simulation.step(None, &mut world, &mut encoder).unwrap();
    }
    let truth = simulation.truth();
    assert!(
        (truth.altitude_m - GROUND_M - 0.12).abs() < 0.02,
        "{}",
        truth.altitude_m
    );
    assert!((truth.attitude.yaw.to_degrees() - N51.heading_deg).abs() < 0.1);
    assert!(truth.attitude.roll.abs() < 0.01 && truth.attitude.pitch.abs() < 0.01);
    // At rest the accelerometer reads the ground holding 1 g up.
    assert!(
        (truth.specific_force_frd[2] + 9.80).abs() < 0.05,
        "{:?}",
        truth.specific_force_frd
    );
    assert!((truth.latitude_deg - N51.latitude_deg).abs() < 1e-6);
}

#[test]
fn full_motor_commands_lift_the_x500_and_disarmed_ones_do_not() {
    let mut world = world();
    let mut encoder = Encoder::default();
    let mut idle = x500(&mut world);
    for _ in 0..250 {
        idle.step(Some(&controls(1.0, false)), &mut world, &mut encoder)
            .unwrap();
    }
    assert!(idle.truth().altitude_m < GROUND_M + 0.2);
    let mut climbing = x500(&mut world);
    for _ in 0..250 {
        climbing
            .step(Some(&controls(1.0, true)), &mut world, &mut encoder)
            .unwrap();
    }
    // 1.74 thrust-to-weight for 1 s: about 3.6 m less the spin-up.
    let truth = climbing.truth();
    let climbed = truth.altitude_m - GROUND_M;
    assert!((2.0..5.0).contains(&climbed), "{climbed} m");
    assert!(
        truth.attitude.roll.abs() < 0.01,
        "a level climb: {:?}",
        truth.attitude
    );
}

#[test]
fn px4_gets_the_imu_and_truth_every_step_and_the_rest_at_their_rates() {
    let mut world = world();
    let mut simulation = x500(&mut world);
    let mut encoder = Encoder::default();
    let mut counts = [0usize; 3];
    for _ in 0..250 {
        for frame in simulation.step(None, &mut world, &mut encoder).unwrap() {
            match message_id(&frame) {
                id if id == HIL_SENSOR.0 => counts[0] += 1,
                id if id == HIL_GPS.0 => counts[1] += 1,
                id if id == HIL_STATE_QUATERNION.0 => counts[2] += 1,
                _ => {}
            }
        }
    }
    // One second: 250 IMU samples, 10 fixes, and truth with every step.
    assert_eq!(counts, [250, 10, 250]);
}

#[test]
fn a_disarmed_x500_stays_put_on_the_ground() {
    let mut world = world();
    let mut simulation = x500(&mut world);
    let mut encoder = Encoder::default();
    for _ in 0..250 {
        simulation.step(None, &mut world, &mut encoder).unwrap();
    }
    let before = simulation.truth();
    // One minute at rest.
    for _ in 0..15_000 {
        simulation
            .step(Some(&controls(0.0, false)), &mut world, &mut encoder)
            .unwrap();
    }
    let after = simulation.truth();
    let yaw_drift = (after.attitude.yaw - before.attitude.yaw).to_degrees();
    let (per_lat, per_lon) = pilotage_world::metres_per_degree(N51.latitude_deg);
    let moved = ((after.latitude_deg - before.latitude_deg) * per_lat)
        .hypot((after.longitude_deg - before.longitude_deg) * per_lon);
    assert!(yaw_drift.abs() < 0.1, "yawed {yaw_drift} deg at rest");
    assert!(moved < 0.01, "slid {moved} m at rest");
    // An inertial rate at rest is the Earth's rotation, 7.3e-5 rad/s.
    for rate in after.rates_frd {
        assert!(rate.abs() < 1e-4, "rates {:?} at rest", after.rates_frd);
    }
}

/// A powered landing rings the stiff leg contacts at the step rate when
/// JSBSim steps once per 4 ms sensor period; at 1 kHz they settle.
#[test]
fn a_powered_landing_comes_to_rest_without_ringing() {
    for heading in [0.0, -45.0] {
        let mut world = world();
        let paths = ModelPaths {
            root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
            aircraft_dir: Aircraft::X500.bundled_dir(),
            model: "x500".to_owned(),
        };
        let start = Start {
            latitude_deg: N51.latitude_deg,
            longitude_deg: N51.longitude_deg,
            heading_deg: heading,
        };
        let mut simulation = Simulation::new(
            Aircraft::X500,
            &paths,
            &mut world,
            start,
            3,
            DEFAULT_NOISE,
            None,
        )
        .unwrap();
        let mut encoder = Encoder::default();
        let mut spin = controls(0.85, true);
        spin.controls[2] = 0.80;
        spin.controls[3] = 0.80;
        // Climb yawing, settle back under less than hover thrust, idle,
        // and disarm.
        for (command, steps) in [
            (spin, 250),
            (controls(0.66, true), 1_000),
            (controls(0.0, true), 500),
        ] {
            for _ in 0..steps {
                simulation
                    .step(Some(&command), &mut world, &mut encoder)
                    .unwrap();
            }
        }
        let disarmed = controls(0.0, false);
        for _ in 0..1_250 {
            simulation
                .step(Some(&disarmed), &mut world, &mut encoder)
                .unwrap();
        }
        let before = simulation.truth();
        let mut worst = 0.0f64;
        for _ in 0..2_500 {
            simulation
                .step(Some(&disarmed), &mut world, &mut encoder)
                .unwrap();
            let rates = simulation.truth().rates_frd;
            worst = worst.max(rates[0].abs()).max(rates[1].abs());
        }
        let after = simulation.truth();
        let yaw_drift = (after.attitude.yaw - before.attitude.yaw).to_degrees();
        assert!(worst < 1e-3, "heading {heading}: rings at {worst} rad/s");
        assert!(
            yaw_drift.abs() < 0.05,
            "heading {heading}: yawed {yaw_drift} deg at rest"
        );
        assert!((after.altitude_m - GROUND_M - 0.12).abs() < 0.02);
    }
}

fn rascal(world: &mut World<UniformTerrain>) -> Simulation {
    let paths = ModelPaths {
        root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
        aircraft_dir: Aircraft::Rascal.bundled_dir(),
        model: "rascal".to_owned(),
    };
    let start = Start {
        latitude_deg: N51.latitude_deg,
        longitude_deg: N51.longitude_deg,
        heading_deg: N51.heading_deg,
    };
    Simulation::new(
        Aircraft::Rascal,
        &paths,
        world,
        start,
        3,
        DEFAULT_NOISE,
        None,
    )
    .unwrap()
}

/// PX4's airframe 1033 outputs: rudder on 3, throttle on 5 (0-based 2, 4).
fn plane_controls(throttle: f32, rudder: f32) -> ActuatorControls {
    let mut plane = controls(0.0, true);
    plane.controls[4] = throttle;
    plane.controls[2] = rudder;
    plane
}

#[test]
fn the_rascal_rests_on_its_gear_at_the_runway_heading() {
    let mut world = world();
    let mut simulation = rascal(&mut world);
    let mut encoder = Encoder::default();
    for _ in 0..1_250 {
        simulation.step(None, &mut world, &mut encoder).unwrap();
    }
    let truth = simulation.truth();
    let height = truth.altitude_m - GROUND_M;
    assert!((0.2..0.5).contains(&height), "the CG stands {height} m up");
    assert!((truth.attitude.yaw.to_degrees() - N51.heading_deg).abs() < 0.5);
    assert!(
        truth.velocity_ned.iter().all(|v| v.abs() < 0.01),
        "{:?}",
        truth.velocity_ned
    );
}

/// PX4 steers the takeoff roll with its yaw output, which drives the
/// rudder and the tail wheel: a right output turns the roll right.
#[test]
fn the_rascal_steers_its_roll_with_px4s_yaw_output() {
    let mut headings = Vec::new();
    for rudder in [-0.5, 0.5] {
        let mut world = world();
        let mut simulation = rascal(&mut world);
        let mut encoder = Encoder::default();
        for step in 0..1_250 {
            let throttle = (step as f32 / 500.0).min(1.0) * 0.3;
            simulation
                .step(
                    Some(&plane_controls(throttle, rudder)),
                    &mut world,
                    &mut encoder,
                )
                .unwrap();
        }
        headings.push(simulation.truth().attitude.yaw.to_degrees());
    }
    let turned = headings[1] - headings[0];
    assert!(
        turned > 3.0,
        "right minus left output turned {turned} deg: {headings:?}"
    );
}

#[test]
fn px4s_gimbal_outputs_point_the_gimbal() {
    let mut world = world();
    let paths = ModelPaths {
        root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
        aircraft_dir: Aircraft::X500.bundled_dir(),
        model: "x500".to_owned(),
    };
    let start = Start {
        latitude_deg: N51.latitude_deg,
        longitude_deg: N51.longitude_deg,
        heading_deg: N51.heading_deg,
    };
    let limits = crate::gimbal::GimbalLimits::from_px4(180.0, 45.0, -135.0, 720.0);
    let mut simulation = Simulation::new(
        Aircraft::X500,
        &paths,
        &mut world,
        start,
        3,
        DEFAULT_NOISE,
        Some(limits),
    )
    .unwrap();
    let mut encoder = Encoder::default();
    // Motors at idle, gimbal pitched to -45 and yawed to 90 degrees.
    let mut pointing = controls(0.0, true);
    pointing.controls[5] = -1.0 / 3.0;
    pointing.controls[6] = 0.25;
    for _ in 0..250 {
        simulation
            .step(Some(&pointing), &mut world, &mut encoder)
            .unwrap();
    }
    let [roll, pitch, yaw] = simulation.gimbal_angles_deg().unwrap();
    assert!(roll.abs() < 0.1 && (pitch + 45.0).abs() < 0.1 && (yaw - 90.0).abs() < 0.1);
    // Disarmed, PX4's outputs are zero and the gimbal holds.
    for _ in 0..250 {
        simulation
            .step(Some(&controls(0.0, false)), &mut world, &mut encoder)
            .unwrap();
    }
    assert!((simulation.gimbal_angles_deg().unwrap()[1] + 45.0).abs() < 0.1);
}
