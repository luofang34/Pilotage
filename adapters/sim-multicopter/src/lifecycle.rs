//! The flight lifecycle: arm, takeoff, flight, landing, and disarm.
//!
//! The shapes follow the DJI Mini 3: Takeoff starts from stopped motors,
//! climbs to [`TAKEOFF_HEIGHT_M`], and hovers; Land descends and stops the
//! motors on the ground. Arm and Disarm are the simulator's mission path:
//! an armed vehicle on the ground lifts off on a climb command.

use pilotage_protocol::{LandedState, MotorState};

use crate::battery::Draw;
use crate::config::Limits;
use crate::kinematics::{BodyVelocity, Kinematics};

/// The hover height after a native takeoff, metres.
pub const TAKEOFF_HEIGHT_M: f64 = 1.2;
/// The highest climb speed of a native takeoff, metres per second.
const TAKEOFF_CLIMB_MPS: f64 = 1.0;
/// The takeoff ends within this distance of its hover height, metres.
const TAKEOFF_CAPTURE_M: f64 = 0.05;
/// The descent speed of a native landing, metres per second.
const LANDING_DESCENT_MPS: f64 = 1.0;

/// The phase of the flight lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightPhase {
    /// Motors stopped on the ground.
    Disarmed,
    /// Motors running on the ground.
    ArmedOnGround,
    /// The native takeoff climbs to its hover height.
    TakingOff,
    /// In flight under velocity control.
    Airborne,
    /// The native landing descends to the ground.
    Landing,
}

impl FlightPhase {
    /// Whether the motors run.
    #[must_use]
    pub fn armed(self) -> bool {
        self != Self::Disarmed
    }

    /// Whether the vehicle is off the ground or leaving it.
    #[must_use]
    pub fn flying(self) -> bool {
        matches!(self, Self::TakingOff | Self::Airborne | Self::Landing)
    }

    /// The landed-detector state the phase reports.
    #[must_use]
    pub fn landed_state(self) -> LandedState {
        match self {
            Self::Disarmed | Self::ArmedOnGround => LandedState::OnGround,
            Self::TakingOff => LandedState::TakingOff,
            Self::Airborne => LandedState::InAir,
            Self::Landing => LandedState::Landing,
        }
    }

    /// The motor state the phase reports.
    #[must_use]
    pub fn motor_state(self) -> MotorState {
        if self.armed() {
            MotorState::Running
        } else {
            MotorState::Stopped
        }
    }

    /// The battery draw of the phase.
    #[must_use]
    pub fn draw(self) -> Draw {
        match self {
            Self::Disarmed => Draw::Off,
            Self::ArmedOnGround => Draw::Idle,
            Self::TakingOff | Self::Airborne | Self::Landing => Draw::Flying,
        }
    }

    /// A short mode name for logs.
    #[must_use]
    pub fn mode_name(self) -> &'static str {
        match self {
            Self::Disarmed => "disarmed",
            Self::ArmedOnGround => "armed-on-ground",
            Self::TakingOff => "takeoff",
            Self::Airborne => "velocity",
            Self::Landing => "landing",
        }
    }

    /// Arms the motors on the ground.
    ///
    /// # Errors
    /// Returns the refusal detail when the vehicle cannot arm.
    pub fn arm(&mut self) -> Result<(), &'static str> {
        match self {
            Self::Disarmed => {
                *self = Self::ArmedOnGround;
                Ok(())
            }
            Self::ArmedOnGround | Self::TakingOff | Self::Airborne | Self::Landing => Ok(()),
        }
    }

    /// Stops the motors on the ground.
    ///
    /// # Errors
    /// Returns the refusal detail in flight.
    pub fn disarm(&mut self) -> Result<(), &'static str> {
        match self {
            Self::Disarmed | Self::ArmedOnGround => {
                *self = Self::Disarmed;
                Ok(())
            }
            Self::TakingOff | Self::Airborne | Self::Landing => {
                Err("the vehicle is in flight; land first")
            }
        }
    }

    /// Starts the native takeoff from the ground.
    ///
    /// # Errors
    /// Returns the refusal detail when the vehicle is not on the ground.
    pub fn takeoff(&mut self) -> Result<(), &'static str> {
        match self {
            Self::Disarmed | Self::ArmedOnGround => {
                *self = Self::TakingOff;
                Ok(())
            }
            Self::TakingOff | Self::Airborne | Self::Landing => {
                Err("the vehicle is not on the ground")
            }
        }
    }

    /// Starts the native landing.
    ///
    /// # Errors
    /// Returns the refusal detail when the vehicle is on the ground.
    pub fn land(&mut self) -> Result<(), &'static str> {
        match self {
            Self::TakingOff | Self::Airborne => {
                *self = Self::Landing;
                Ok(())
            }
            Self::Landing => Ok(()),
            Self::Disarmed | Self::ArmedOnGround => Err("the vehicle is on the ground"),
        }
    }

    /// Whether the phase accepts a velocity command.
    ///
    /// # Errors
    /// Returns the refusal detail when it does not.
    pub fn admits_velocity(self) -> Result<(), &'static str> {
        match self {
            Self::Disarmed => Err("the motors are stopped; arm or take off first"),
            Self::Landing => Err("the native landing is in progress"),
            Self::ArmedOnGround | Self::TakingOff | Self::Airborne => Ok(()),
        }
    }

    /// Advances the vehicle by `dt_s` under `command` and moves the phase
    /// on from what the vehicle did.
    pub fn step(
        &mut self,
        body: &mut Kinematics,
        command: BodyVelocity,
        limits: &Limits,
        dt_s: f64,
    ) {
        match self {
            Self::Disarmed => body.hold_on_ground(),
            Self::ArmedOnGround => {
                if command.down < 0.0 {
                    *self = Self::Airborne;
                    body.step(command, limits, dt_s);
                } else {
                    body.hold_on_ground();
                }
            }
            Self::TakingOff => {
                let remaining = TAKEOFF_HEIGHT_M - body.height_m();
                // A unit gain with the 0.5 s velocity lag is well damped.
                let climb = remaining.clamp(-TAKEOFF_CLIMB_MPS, TAKEOFF_CLIMB_MPS);
                // A faster commanded climb wins; nothing else stops the takeoff.
                let down = if command.down < -climb {
                    command.down
                } else {
                    -climb
                };
                body.step(BodyVelocity { down, ..command }, limits, dt_s);
                let remaining = TAKEOFF_HEIGHT_M - body.height_m();
                if remaining.abs() <= TAKEOFF_CAPTURE_M && body.vel_ned_mps[2].abs() < 0.1 {
                    *self = Self::Airborne;
                }
            }
            Self::Airborne => {
                if body.step(command, limits, dt_s) {
                    *self = Self::ArmedOnGround;
                    body.hold_on_ground();
                }
            }
            Self::Landing => {
                let descent = BodyVelocity {
                    down: LANDING_DESCENT_MPS,
                    ..BodyVelocity::default()
                };
                if body.step(descent, limits, dt_s) {
                    *self = Self::Disarmed;
                    body.hold_on_ground();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
