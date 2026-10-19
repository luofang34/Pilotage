//! One simulated vehicle: a JSBSim engine on the world's ground, its
//! actuators, and its sensors, stepped one sensor period at a time.

use pilotage_world::{TerrainSource, World, WorldError};

use crate::aircraft::{Actuation, Aircraft};
use crate::engine::{Engine, EngineError, ModelPaths};
use crate::gimbal::{GIMBAL_OUTPUTS, Gimbal, GimbalLimits};
use crate::ground::{GroundPatch, HALF_CELLS};
use crate::hil::{
    ActuatorControls, Encoder, FIELDS_BARO, FIELDS_DIFF_PRESSURE, FIELDS_IMU, FIELDS_MAG,
};
use crate::sensors::{NoiseLevels, Sensors};
use crate::truth::{Truth, TruthProperties};

/// The sensor period: PX4's simulator rate of 250 Hz.
pub const STEP_S: f64 = 0.004;
/// JSBSim steps per sensor period. The landing legs' spring and damper
/// are stiff: at one 4 ms step their explicit integration rings at the
/// step rate on the ground, so JSBSim runs at 1 kHz.
const SUBSTEPS: u32 = 4;
/// The JSBSim step, s.
const PHYSICS_DT_S: f64 = STEP_S / SUBSTEPS as f64;
/// Steps per magnetometer sample (125 Hz).
const MAG_EVERY: u64 = 2;
/// Steps per barometer and airspeed sample (50 Hz).
const BARO_EVERY: u64 = 5;
/// Steps per GNSS fix (10 Hz).
const GNSS_EVERY: u64 = 25;
/// Steps per heartbeat (1 Hz).
const HEARTBEAT_EVERY: u64 = 250;

/// Where and how a vehicle starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Start {
    /// Latitude, degrees.
    pub latitude_deg: f64,
    /// Longitude, degrees.
    pub longitude_deg: f64,
    /// True heading, degrees.
    pub heading_deg: f64,
}

/// Why a simulation could not start or continue.
#[derive(Debug, thiserror::Error)]
pub enum SimulationError {
    /// JSBSim failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The world could not answer.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// One vehicle in the world.
pub struct Simulation {
    engine: Engine,
    truth: TruthProperties,
    actuation: Actuation,
    sensors: Sensors,
    patch: GroundPatch,
    gimbal: Option<Gimbal>,
    steps: u64,
}

impl Simulation {
    /// Loads `aircraft`, stands it on the world's ground at `start`, and
    /// settles it there.
    ///
    /// # Errors
    ///
    /// Returns [`SimulationError`] when JSBSim or the world fails.
    pub fn new<T: TerrainSource>(
        aircraft: Aircraft,
        paths: &ModelPaths,
        world: &mut World<T>,
        start: Start,
        seed: u64,
        noise: NoiseLevels,
        gimbal: Option<GimbalLimits>,
    ) -> Result<Self, SimulationError> {
        let mut engine = Engine::load(paths, PHYSICS_DT_S)?;
        let patch = GroundPatch::sample_blocking(
            world,
            start.latitude_deg,
            start.longitude_deg,
            HALF_CELLS,
        )?;
        engine.set_ground(Box::new(patch.clone()))?;
        for (name, value) in [
            ("ic/lat-geod-deg", start.latitude_deg),
            ("ic/long-gc-deg", start.longitude_deg),
            ("ic/psi-true-deg", start.heading_deg),
            ("ic/h-agl-ft", 0.0),
            ("ic/u-fps", 0.0),
            ("ic/v-fps", 0.0),
            ("ic/w-fps", 0.0),
        ] {
            let property = engine.input(name)?;
            engine.set(property, value);
        }
        engine.run_ic()?;
        // Ground trim: rest the gear on the terrain at the start attitude.
        let trim = engine.input("simulation/do_simple_trim")?;
        engine.command(trim, "simulation/do_simple_trim", 2.0)?;
        let truth = TruthProperties::resolve(&mut engine)?;
        let actuation = Actuation::resolve(aircraft, &mut engine)?;
        Ok(Self {
            engine,
            truth,
            actuation,
            sensors: Sensors::new(seed, noise),
            patch,
            gimbal: gimbal.map(Gimbal::new),
            steps: 0,
        })
    }

    /// The gimbal's angles relative to the body, degrees, if it has one.
    pub fn gimbal_angles_deg(&self) -> Option<[f64; 3]> {
        self.gimbal.as_ref().map(Gimbal::angles_deg)
    }

    /// The vehicle's true state now.
    pub fn truth(&self) -> Truth {
        self.truth.read(&self.engine)
    }

    /// Advances one step under `controls`, and returns the frames PX4
    /// reads for it: the IMU every step, the other sensors at their rates.
    ///
    /// # Errors
    ///
    /// Returns [`SimulationError`] when JSBSim or the world fails.
    pub fn step<T: TerrainSource>(
        &mut self,
        controls: Option<&ActuatorControls>,
        world: &mut World<T>,
        encoder: &mut Encoder,
    ) -> Result<Vec<Vec<u8>>, SimulationError> {
        for _ in 0..SUBSTEPS {
            self.actuation
                .apply(&mut self.engine, controls, PHYSICS_DT_S);
            self.engine.step()?;
        }
        self.steps = self.steps.wrapping_add(1);
        if let Some(gimbal) = &mut self.gimbal {
            let outputs = controls
                .filter(|c| c.armed())
                .map(|c| GIMBAL_OUTPUTS.map(|i| c.controls.get(i).map_or(0.0, |&v| f64::from(v))));
            gimbal.step(outputs, STEP_S);
        }
        let truth = self.truth();
        if self
            .patch
            .needs_recentre(truth.latitude_deg, truth.longitude_deg)
        {
            self.patch = GroundPatch::sample_blocking(
                world,
                truth.latitude_deg,
                truth.longitude_deg,
                HALF_CELLS,
            )?;
            self.engine.replace_ground(Box::new(self.patch.clone()));
            tracing::debug!(
                latitude_deg = truth.latitude_deg,
                longitude_deg = truth.longitude_deg,
                "ground grid sampled around the vehicle"
            );
        }
        let due = |every: u64| self.steps.is_multiple_of(every);
        let mut fields = FIELDS_IMU;
        if due(MAG_EVERY) {
            fields |= FIELDS_MAG;
        }
        if due(BARO_EVERY) {
            fields |= FIELDS_BARO | FIELDS_DIFF_PRESSURE;
        }
        let mut frames = Vec::with_capacity(4);
        if self.steps == 1 || due(HEARTBEAT_EVERY) {
            frames.push(encoder.heartbeat());
        }
        let inertial = self.sensors.inertial(&truth, world)?;
        frames.push(encoder.hil_sensor(&inertial, fields));
        if due(GNSS_EVERY) {
            frames.push(encoder.hil_gps(&self.sensors.gnss(&truth)));
        }
        // Truth every step: a flight controller that synthesizes its own
        // sensors (Aviate's JSBSim board) steps on it.
        frames.push(encoder.hil_state_quaternion(&truth));
        Ok(frames)
    }
}

#[cfg(test)]
mod tests;
