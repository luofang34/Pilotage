//! The aircraft the sidecar flies, and how PX4's actuator outputs drive
//! each one.
//!
//! The x500 is this crate's model (`aircraft/x500`), flown through the
//! motor model with PX4's quad-X rotor order of airframe 3010. The
//! Rascal 110 (`aircraft/rascal`) is the JSBSim model PX4 ships for its
//! airframe 1033, with a tail wheel that steers, flown with that
//! airframe's channel order: rudder and tail wheel on output 3, throttle
//! on 5, aileron on 6, elevator on 8.

use std::path::{Path, PathBuf};

use crate::engine::{Engine, EngineError, Property};
use crate::hil::ActuatorControls;
use crate::motor::{QuadMotors, X500_MOTOR};

const LBF_PER_N: f64 = 1.0 / 4.448_221_615_260_5;
const LBF_FT_PER_N_M: f64 = LBF_PER_N / 0.3048;

/// An aircraft the sidecar can fly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aircraft {
    /// An x500-class quadcopter.
    X500,
    /// The Rascal 110 fixed-wing.
    Rascal,
}

/// The names `--aircraft` takes.
pub const AIRCRAFT_NAMES: [&str; 2] = ["x500", "rascal"];

/// One output channel driving one JSBSim property.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Channel {
    output: usize,
    property: &'static str,
    scale: f64,
}

/// The Rascal's channels, as PX4's own JSBSim bridge maps them.
const RASCAL_CHANNELS: [Channel; 4] = [
    Channel {
        output: 2,
        property: "fcs/rudder-cmd-norm",
        scale: -1.0,
    },
    Channel {
        output: 4,
        property: "fcs/throttle-cmd-norm",
        scale: 1.0,
    },
    Channel {
        output: 5,
        property: "fcs/aileron-cmd-norm",
        scale: -1.0,
    },
    Channel {
        output: 7,
        property: "fcs/elevator-cmd-norm",
        scale: -1.0,
    },
];

impl Aircraft {
    /// Parses an `--aircraft` name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "x500" => Some(Self::X500),
            "rascal" => Some(Self::Rascal),
            _ => None,
        }
    }

    /// The JSBSim model name: its XML file's stem.
    pub fn model(self) -> &'static str {
        match self {
            Self::X500 => "x500",
            Self::Rascal => "rascal",
        }
    }

    /// The height of the centre of gravity above level ground when the
    /// aircraft rests on its gear, m.
    pub fn rest_height_m(self) -> f64 {
        match self {
            Self::X500 => 0.12,
            Self::Rascal => 0.31,
        }
    }

    /// The model's directory in this crate.
    pub fn bundled_dir(self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("aircraft")
            .join(self.model())
    }
}

/// How PX4's outputs reach one engine's model.
pub enum Actuation {
    /// Four rotors through the motor model.
    Quad {
        /// The rotor speeds.
        motors: QuadMotors,
        /// Each rotor's thrust magnitude, lbf.
        thrust: [Property; 4],
        /// The summed drag torque, lbf·ft.
        torque: Property,
    },
    /// Control surfaces and a throttle set directly.
    Channels(Vec<(usize, Property, f64)>),
}

impl Actuation {
    /// Resolves the model's actuator properties.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the model lacks one.
    pub fn resolve(aircraft: Aircraft, engine: &mut Engine) -> Result<Self, EngineError> {
        Ok(match aircraft {
            Aircraft::X500 => Self::Quad {
                motors: QuadMotors::new(X500_MOTOR),
                thrust: [
                    engine.input("external_reactions/rotor0/magnitude")?,
                    engine.input("external_reactions/rotor1/magnitude")?,
                    engine.input("external_reactions/rotor2/magnitude")?,
                    engine.input("external_reactions/rotor3/magnitude")?,
                ],
                torque: engine.input("external_reactions/rotor_drag/magnitude-lbsft")?,
            },
            Aircraft::Rascal => Self::Channels(
                RASCAL_CHANNELS
                    .iter()
                    .map(|c| Ok((c.output, engine.property(c.property)?, c.scale)))
                    .collect::<Result<_, EngineError>>()?,
            ),
        })
    }

    /// Applies the latest outputs (none yet: everything off) for a step
    /// of `dt_s`.
    pub fn apply(&mut self, engine: &mut Engine, controls: Option<&ActuatorControls>, dt_s: f64) {
        let armed = controls.is_some_and(ActuatorControls::armed);
        let output = |i: usize| {
            controls
                .and_then(|c| c.controls.get(i))
                .map_or(0.0, |&v| f64::from(v))
        };
        match self {
            Self::Quad {
                motors,
                thrust,
                torque,
            } => {
                let rotors = motors.step([output(0), output(1), output(2), output(3)], armed, dt_s);
                for (property, newtons) in thrust.iter().zip(rotors.thrust_n) {
                    engine.set(*property, newtons * LBF_PER_N);
                }
                engine.set(*torque, rotors.yaw_torque_nm * LBF_FT_PER_N_M);
            }
            Self::Channels(channels) => {
                for &(channel, property, scale) in channels.iter() {
                    let value = if armed { output(channel) * scale } else { 0.0 };
                    engine.set(property, if value.is_finite() { value } else { 0.0 });
                }
            }
        }
    }
}
