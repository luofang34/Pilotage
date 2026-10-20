//! The engine over the real JSBSim library: loading, properties, and
//! the ground callback.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use super::{Engine, EngineError, ModelPaths};
use crate::ground::{GroundAnswer, GroundModel};

fn x500() -> ModelPaths {
    ModelPaths {
        root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
        aircraft_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("aircraft/x500"),
        model: "x500".to_owned(),
    }
}

/// Flat ground at one height.
struct Flat(f64);

impl GroundModel for Flat {
    fn height(&self, lat_rad: f64, lon_rad: f64) -> Option<GroundAnswer> {
        Some(GroundAnswer {
            height_m: self.0,
            slope_north: 0.0,
            slope_east: 0.0,
            normal_ecef: crate::ground::surface_normal_ecef(
                lat_rad.to_degrees(),
                lon_rad.to_degrees(),
                self.0,
                0.0,
                0.0,
            )?,
        })
    }
}

fn settle(engine: &mut Engine, seconds: f64) {
    let steps = (seconds / 0.004) as usize;
    for _ in 0..steps {
        engine.step().unwrap();
    }
}

fn start(engine: &mut Engine, ground: f64) {
    engine.set_ground(Box::new(Flat(ground))).unwrap();
    for (name, value) in [
        ("ic/lat-geod-deg", 40.5),
        ("ic/long-gc-deg", -74.7),
        ("ic/h-agl-ft", 0.5),
    ] {
        let property = engine.input(name).unwrap();
        engine.set(property, value);
    }
    engine.run_ic().unwrap();
}

#[test]
fn a_missing_model_is_a_load_error_naming_it() {
    let mut paths = x500();
    paths.model = "no-such-aircraft".to_owned();
    match Engine::load(&paths, 0.004) {
        Err(EngineError::Load { model, .. }) => assert_eq!(model, "no-such-aircraft"),
        Err(other) => panic!("expected a load error, got {other}"),
        Ok(_) => panic!("a missing model loaded"),
    }
}

#[test]
fn an_unknown_property_is_refused_and_an_input_is_created() {
    let mut engine = Engine::load(&x500(), 0.004).unwrap();
    assert!(matches!(
        engine.property("no/such-property"),
        Err(EngineError::NoProperty { .. })
    ));
    let input = engine.input("pilotage/test-input").unwrap();
    engine.set(input, 4.25);
    assert_eq!(engine.get(input), 4.25);
    assert!(matches!(
        engine.property("bad\0name"),
        Err(EngineError::Nul { .. })
    ));
}

/// The x500 drops onto ground that only the callback knows about, and
/// rests with its legs on it: the centre of gravity 0.12 m above.
#[test]
fn the_vehicle_rests_on_the_ground_the_callback_answers() {
    for ground in [0.0, 312.5] {
        let mut engine = Engine::load(&x500(), 0.004).unwrap();
        start(&mut engine, ground);
        settle(&mut engine, 3.0);
        let altitude = engine.property("position/h-sl-meters").unwrap();
        let height = engine.get(altitude) - ground;
        assert!(
            (height - 0.12).abs() < 0.02,
            "rests {height} m above {ground} m"
        );
        let agl = engine.property("position/h-agl-ft").unwrap();
        assert!((engine.get(agl) * 0.3048 - 0.12).abs() < 0.02);
    }
}

#[test]
fn replacing_the_ground_moves_the_resting_height() {
    let mut engine = Engine::load(&x500(), 0.004).unwrap();
    start(&mut engine, 10.0);
    settle(&mut engine, 2.0);
    assert!(engine.replace_ground(Box::new(Flat(9.0))));
    settle(&mut engine, 3.0);
    let altitude = engine.property("position/h-sl-meters").unwrap();
    assert!(
        (engine.get(altitude) - 9.12).abs() < 0.02,
        "{}",
        engine.get(altitude)
    );
}

/// JSBSim keeps process-wide state that loading a model writes: the
/// first `Element` built fills the unit-conversion table while other
/// threads already read it. Engines loaded and flown on many threads at
/// once must not race on it. Without the engine's own serialisation a run
/// of this test crashes (SIGSEGV) or hangs in a few percent of processes,
/// since the race is on the process's first model load.
#[test]
fn engines_load_and_fly_on_many_threads_at_once() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 4;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
    let workers: Vec<_> = (0..THREADS)
        .map(|_| {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                for _ in 0..ROUNDS {
                    barrier.wait();
                    let mut engine = Engine::load(&x500(), 0.004).expect("the x500 loads");
                    start(&mut engine, 10.0);
                    settle(&mut engine, 0.2);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("an engine thread finished");
    }
}
