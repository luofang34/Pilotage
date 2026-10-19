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

/// The JSBSim step, s. The landing legs' spring and damper are stiff: at
/// one 4 ms step their explicit integration rings on the ground, so
/// JSBSim always runs at 1 kHz, several steps per state step when the
/// state step is longer.
const PHYSICS_DT_S: f64 = 0.001;
/// The state steps a flight controller can run on, ms: each divides every
/// sensor period below, so every sensor keeps its rate.
pub const STEP_MS_CHOICES: [u32; 3] = [1, 2, 4];
/// Magnetometer period, ms (125 Hz).
const MAG_PERIOD_MS: u64 = 8;
/// Barometer and airspeed period, ms (50 Hz).
const BARO_PERIOD_MS: u64 = 20;
/// GNSS period, ms (10 Hz).
const GNSS_PERIOD_MS: u64 = 100;
/// Heartbeat period, ms (1 Hz).
const HEARTBEAT_PERIOD_MS: u64 = 1_000;

/// What one simulated vehicle is and how it is stepped.
#[derive(Debug, Clone, PartialEq)]
pub struct Vehicle {
    /// The aircraft.
    pub aircraft: Aircraft,
    /// Where its model lives.
    pub paths: ModelPaths,
    /// Where it starts.
    pub start: Start,
    /// The sensor noise seed.
    pub seed: u64,
    /// The sensor noise.
    pub noise: NoiseLevels,
    /// The gimbal's travel, when it carries one.
    pub gimbal: Option<GimbalLimits>,
    /// The state step, ms: one of [`STEP_MS_CHOICES`]. PX4 takes 4 (its
    /// 250 Hz simulator rate); Aviate's JSBSim board takes 1.
    pub step_ms: u32,
}

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
    /// A position is not well formed for the world model.
    #[error("position: {0}")]
    Position(#[from] pilotage_world::PositionError),
}

/// One vehicle in the world.
pub struct Simulation {
    engine: Engine,
    truth: TruthProperties,
    actuation: Actuation,
    sensors: Sensors,
    patch: GroundPatch,
    gimbal: Option<Gimbal>,
    step_ms: u32,
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
        vehicle: &Vehicle,
        world: &mut World<T>,
    ) -> Result<Self, SimulationError> {
        let start = vehicle.start;
        let mut engine = Engine::load(&vehicle.paths, PHYSICS_DT_S)?;
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
        let actuation = Actuation::resolve(vehicle.aircraft, &mut engine)?;
        Ok(Self {
            engine,
            truth,
            actuation,
            sensors: Sensors::new(vehicle.seed, vehicle.noise),
            patch,
            gimbal: vehicle.gimbal.map(Gimbal::new),
            step_ms: vehicle.step_ms.clamp(1, 4),
            steps: 0,
        })
    }

    /// The state step, s.
    pub fn step_s(&self) -> f64 {
        f64::from(self.step_ms) * 1e-3
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
        for _ in 0..self.step_ms {
            self.actuation
                .apply(&mut self.engine, controls, PHYSICS_DT_S);
            self.engine.step()?;
        }
        self.steps = self.steps.wrapping_add(1);
        let step_s = self.step_s();
        if let Some(gimbal) = &mut self.gimbal {
            let outputs = controls
                .filter(|c| c.armed())
                .map(|c| GIMBAL_OUTPUTS.map(|i| c.controls.get(i).map_or(0.0, |&v| f64::from(v))));
            gimbal.step(outputs, step_s);
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
        let elapsed_ms = self.steps.wrapping_mul(u64::from(self.step_ms));
        let due = |period_ms: u64| elapsed_ms.is_multiple_of(period_ms);
        let mut fields = FIELDS_IMU;
        if due(MAG_PERIOD_MS) {
            fields |= FIELDS_MAG;
        }
        if due(BARO_PERIOD_MS) {
            fields |= FIELDS_BARO | FIELDS_DIFF_PRESSURE;
        }
        let mut frames = Vec::with_capacity(4);
        if self.steps == 1 || due(HEARTBEAT_PERIOD_MS) {
            frames.push(encoder.heartbeat());
        }
        let inertial = self.sensors.inertial(&truth, world)?;
        frames.push(encoder.hil_sensor(&inertial, fields));
        if due(GNSS_PERIOD_MS) {
            frames.push(encoder.hil_gps(&self.sensors.gnss(&truth)?));
        }
        // Truth every step: a flight controller that synthesizes its own
        // sensors (Aviate's JSBSim board) steps on it.
        frames.push(encoder.hil_state_quaternion(&truth));
        Ok(frames)
    }
}

#[cfg(test)]
mod tests;
