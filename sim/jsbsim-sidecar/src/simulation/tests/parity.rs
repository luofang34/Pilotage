//! The two engines fly one vehicle the same way: one script of commands,
//! on one world, through the whole simulation, compared at every step.
//! The port's own elementary functions differ from the platform's libm by
//! a few units in the last place, so the tracks are close, not identical.

use std::path::PathBuf;

use super::{
    ActuatorControls, Aircraft, Encoder, EngineKind, GROUND_M, ModelPaths, N51, Simulation, Start,
    Vehicle, controls, plane_controls, vehicle, world,
};
use crate::truth::Truth;

/// The widest the two tracks may part at any step of a flight.
const POSITION_M: f64 = 1e-3;
const VELOCITY_M_S: f64 = 1e-3;
const ATTITUDE_RAD: f64 = 1e-4;
/// Metres per degree of latitude, near enough to bound a difference.
const M_PER_DEG: f64 = 111_000.0;

fn fly(kind: EngineKind, aircraft: Aircraft, script: &[ActuatorControls]) -> Vec<Truth> {
    let mut world = world();
    let paths = ModelPaths {
        root: option_env!("PILOTAGE_JSBSIM_ROOT").map(PathBuf::from),
        aircraft_dir: aircraft.bundled_dir(),
        model: aircraft.model().to_owned(),
    };
    let start = Start {
        latitude_deg: N51.latitude_deg,
        longitude_deg: N51.longitude_deg,
        heading_deg: N51.heading_deg,
    };
    let flown = Vehicle {
        engine: kind,
        ..vehicle(aircraft, paths, start, None, 4)
    };
    let mut simulation = Simulation::new(&flown, &mut world).unwrap();
    let mut encoder = Encoder::default();
    let mut track = vec![simulation.truth()];
    for command in script {
        simulation
            .step(Some(command), &mut world, &mut encoder)
            .unwrap();
        track.push(simulation.truth());
    }
    track
}

fn largest(differences: impl Iterator<Item = f64>) -> f64 {
    differences.fold(0.0, |widest, d| widest.max(d.abs()))
}

fn assert_same_flight(rust: &[Truth], cpp: &[Truth]) {
    assert_eq!(rust.len(), cpp.len());
    for (step, (r, c)) in rust.iter().zip(cpp).enumerate() {
        let position = largest(
            [
                r.altitude_m - c.altitude_m,
                (r.latitude_deg - c.latitude_deg) * M_PER_DEG,
                (r.longitude_deg - c.longitude_deg) * M_PER_DEG,
            ]
            .into_iter(),
        );
        let velocity = largest((0..3).map(|i| r.velocity_ned[i] - c.velocity_ned[i]));
        let attitude = largest(
            [
                r.attitude.roll - c.attitude.roll,
                r.attitude.pitch - c.attitude.pitch,
                r.attitude.yaw - c.attitude.yaw,
            ]
            .into_iter(),
        );
        assert!(
            position < POSITION_M && velocity < VELOCITY_M_S && attitude < ATTITUDE_RAD,
            "step {step}: {position} m, {velocity} m/s, {attitude} rad apart"
        );
    }
}

#[test]
fn both_engines_fly_the_x500_alike() {
    let script: Vec<_> = (0..5_000)
        .map(|step| {
            let mut command = controls(if step < 200 { 1.0 } else { 0.78 }, true);
            // A pitch and a roll pair, so the rates and the drag are in it.
            if (500..520).contains(&step) {
                command.controls[0] += 0.03;
                command.controls[3] += 0.03;
            }
            if (520..540).contains(&step) {
                command.controls[1] += 0.03;
                command.controls[2] += 0.03;
            }
            command
        })
        .collect();
    let rust = fly(EngineKind::Rust, Aircraft::X500, &script);
    let cpp = fly(EngineKind::Cpp, Aircraft::X500, &script);
    let climbed = rust[rust.len() - 1].altitude_m - GROUND_M;
    assert!(climbed > 100.0, "the flight stayed low: {climbed} m");
    assert_same_flight(&rust, &cpp);
}

/// The Rascal flies open loop, so this flight is unstable: it rotates off
/// the roll, banks and porpoises, which any drift between the engines'
/// aerodynamics would amplify.
#[test]
fn both_engines_fly_the_rascal_alike() {
    let script: Vec<_> = (0..5_000)
        .map(|step| {
            let throttle = (step as f32 / 250.0).min(1.0);
            let mut command = plane_controls(if step > 2_000 { 0.7 } else { throttle }, 0.0);
            if step > 1_750 {
                command.controls[7] = 0.16;
            }
            if step > 4_000 {
                command.controls[5] = 0.02;
            }
            command
        })
        .collect();
    let rust = fly(EngineKind::Rust, Aircraft::Rascal, &script);
    let cpp = fly(EngineKind::Cpp, Aircraft::Rascal, &script);
    let peak = largest(rust.iter().map(|t| t.altitude_m - GROUND_M));
    let bank = largest(rust.iter().map(|t| t.attitude.roll));
    assert!(peak > 15.0, "the flight stayed low: {peak} m");
    assert!(bank > 0.3, "the flight stayed level: {bank} rad");
    assert_same_flight(&rust, &cpp);
}
