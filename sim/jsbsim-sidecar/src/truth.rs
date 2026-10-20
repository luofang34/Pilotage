//! The vehicle's true state, read from JSBSim each step in SI units and
//! the body FRD / local NED frames.

use crate::engine::{Engine, EngineError, Property};
use crate::rotation::Euler;

const M_PER_FT: f64 = 0.3048;
const N_PER_LBF: f64 = 4.448_221_615_260_5;
const KG_PER_SLUG: f64 = 14.593_902_937_206_4;
const PA_PER_PSF: f64 = 47.880_258_888_9;

/// The vehicle's true state at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Truth {
    /// Simulation time, s.
    pub time_s: f64,
    /// Geodetic latitude, degrees.
    pub latitude_deg: f64,
    /// Longitude, degrees.
    pub longitude_deg: f64,
    /// Height above mean sea level (the simulator's ellipsoid), m.
    pub altitude_m: f64,
    /// Velocity over the ground, NED, m/s.
    pub velocity_ned: [f64; 3],
    /// Attitude, body FRD to NED.
    pub attitude: Euler,
    /// Angular rate relative to inertial space, body FRD, rad/s: what a
    /// gyroscope measures.
    pub rates_frd: [f64; 3],
    /// Non-gravitational force per unit mass, body FRD, m/s²: what an
    /// accelerometer measures.
    pub specific_force_frd: [f64; 3],
    /// Dynamic pressure, Pa.
    pub dynamic_pressure_pa: f64,
    /// True airspeed, m/s.
    pub true_airspeed_mps: f64,
    /// Calibrated airspeed, m/s.
    pub calibrated_airspeed_mps: f64,
}

/// The JSBSim properties a [`Truth`] is read from.
#[derive(Debug, Clone, Copy)]
pub struct TruthProperties {
    latitude_deg: Property,
    longitude_deg: Property,
    altitude_m: Property,
    velocity_ned_fps: [Property; 3],
    euler_rad: [Property; 3],
    rates_rad_s: [Property; 3],
    forces_lbf: [Property; 3],
    mass_slug: Property,
    qbar_psf: Property,
    vtrue_fps: Property,
    vc_fps: Property,
}

impl TruthProperties {
    /// Resolves every property on `engine`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::NoProperty`] when the model lacks one.
    pub fn resolve(engine: &mut Engine) -> Result<Self, EngineError> {
        let mut three = |names: [&str; 3]| -> Result<[Property; 3], EngineError> {
            Ok([
                engine.property(names[0])?,
                engine.property(names[1])?,
                engine.property(names[2])?,
            ])
        };
        let velocity_ned_fps = three([
            "velocities/v-north-fps",
            "velocities/v-east-fps",
            "velocities/v-down-fps",
        ])?;
        let euler_rad = three(["attitude/phi-rad", "attitude/theta-rad", "attitude/psi-rad"])?;
        let rates_rad_s = three([
            "velocities/pi-rad_sec",
            "velocities/qi-rad_sec",
            "velocities/ri-rad_sec",
        ])?;
        let forces_lbf = three([
            "forces/fbx-total-lbs",
            "forces/fby-total-lbs",
            "forces/fbz-total-lbs",
        ])?;
        Ok(Self {
            latitude_deg: engine.property("position/lat-geod-deg")?,
            longitude_deg: engine.property("position/long-gc-deg")?,
            altitude_m: engine.property("position/h-sl-meters")?,
            velocity_ned_fps,
            euler_rad,
            rates_rad_s,
            forces_lbf,
            mass_slug: engine.property("inertia/mass-slugs")?,
            qbar_psf: engine.property("aero/qbar-psf")?,
            vtrue_fps: engine.property("velocities/vtrue-fps")?,
            vc_fps: engine.property("velocities/vc-fps")?,
        })
    }

    /// Reads the current state.
    pub fn read(&self, engine: &Engine) -> Truth {
        let get3 = |p: [Property; 3], scale: f64| p.map(|p| engine.get(p) * scale);
        let [roll, pitch, yaw] = get3(self.euler_rad, 1.0);
        let mass_kg = engine.get(self.mass_slug) * KG_PER_SLUG;
        let force_n = get3(self.forces_lbf, N_PER_LBF);
        Truth {
            time_s: engine.sim_time_s(),
            latitude_deg: engine.get(self.latitude_deg),
            longitude_deg: engine.get(self.longitude_deg),
            altitude_m: engine.get(self.altitude_m),
            velocity_ned: get3(self.velocity_ned_fps, M_PER_FT),
            attitude: Euler { roll, pitch, yaw },
            rates_frd: get3(self.rates_rad_s, 1.0),
            specific_force_frd: force_n.map(|f| f / mass_kg),
            dynamic_pressure_pa: engine.get(self.qbar_psf) * PA_PER_PSF,
            true_airspeed_mps: engine.get(self.vtrue_fps) * M_PER_FT,
            calibrated_airspeed_mps: engine.get(self.vc_fps) * M_PER_FT,
        }
    }
}
