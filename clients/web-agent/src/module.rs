//! The module state behind the wasm surface, testable without a browser.

use pilotage_agent::{
    AgentFlight, Demand, Directive, Discrete, ModelReply, Phase, Scenario, VehicleOffer,
    VehicleState, yaw_of_quaternion,
};
use serde::{Deserialize, Serialize};

use crate::error::ModuleError;

/// The identity that the agent announces while it is the input source. The
/// headless port announces the same identity.
pub const AGENT_PROFILE_ID: &str = "automation.intent-pilot/v1";

/// `arm_state` value of an armed vehicle in the avionics telemetry.
const ARM_STATE_ARMED: u32 = 2;
/// `arm_state` value of a disarmed vehicle.
const ARM_STATE_DISARMED: u32 = 1;

/// One operational-estimate sample in local NED, as the telemetry ingress of
/// the shell admits it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Telemetry {
    /// Position north, east and down of the local origin, in metres.
    pub position_ned_m: [f64; 3],
    /// Velocity north, east and down, in metres per second.
    pub velocity_ned_mps: [f64; 3],
    /// The body-to-NED quaternion as w, x, y, z.
    pub quaternion_wxyz: [f64; 4],
    /// The arm state code of the flight controller: 0 unknown, 1 disarmed,
    /// 2 armed.
    pub arm_state: u32,
}

/// What the module did with one model reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Decision {
    /// The directive passed the three checks and is now flown.
    Flown {
        /// The directive.
        directive: Directive,
    },
    /// The reply did not pass a check. The vehicle keeps its last directive.
    Refused {
        /// The refusal in words.
        reason: String,
    },
    /// The gateway or the adapter gave no usable reply. The vehicle keeps its
    /// last directive.
    Fault {
        /// The fault in words.
        detail: String,
    },
}

/// The agent's input to the control runtime for one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// The flight demand.
    pub demand: Demand,
    /// True in the tick that asks for an arm.
    pub arm: bool,
    /// True in the tick that asks for a disarm.
    pub disarm: bool,
    /// The executor phase after this tick.
    pub phase: Phase,
}

/// A reply line of a model adapter that reports a fault.
#[derive(Debug, Deserialize)]
struct AdapterFault {
    error: String,
}

/// One engaged agent.
#[derive(Debug)]
pub struct AgentModule {
    flight: AgentFlight,
}

impl AgentModule {
    /// Starts a flight on the chart document, under what the motion scope
    /// advertises.
    pub fn new(
        chart_json: &str,
        max_linear_mps: f64,
        disarm_offered: bool,
    ) -> Result<Self, ModuleError> {
        let scenario = Scenario::parse(chart_json).map_err(ModuleError::Chart)?;
        if !max_linear_mps.is_finite() || max_linear_mps <= 0.0 {
            return Err(ModuleError::SpeedLimit { max_linear_mps });
        }
        let offer = VehicleOffer {
            max_linear_mps,
            disarm_offered,
        };
        Ok(Self {
            flight: AgentFlight::new(&scenario, offer),
        })
    }

    /// The model request for one operator message, as one JSON line.
    pub fn request_json(&self, message: &str) -> Result<String, ModuleError> {
        serde_json::to_string(&self.flight.request(message, Vec::new()))
            .map_err(ModuleError::EncodeRequest)
    }

    /// Takes the reply line of the model gateway for `message`.
    pub fn take_reply_json(&mut self, message: &str, reply_json: &str, now_s: f64) -> Decision {
        if let Ok(fault) = serde_json::from_str::<AdapterFault>(reply_json) {
            return Decision::Fault {
                detail: fault.error,
            };
        }
        let reply: ModelReply = match serde_json::from_str(reply_json) {
            Ok(reply) => reply,
            Err(error) => {
                return Decision::Fault {
                    detail: format!("the reply is not a model reply: {error}"),
                };
            }
        };
        match self.flight.take_reply(message, &reply, now_s) {
            Ok(directive) => Decision::Flown { directive },
            Err(refusal) => Decision::Refused {
                reason: refusal.to_string(),
            },
        }
    }

    /// Records one telemetry sample.
    pub fn observe(&mut self, telemetry: Telemetry) {
        let [north_m, east_m, down_m] = telemetry.position_ned_m;
        let [vel_north_mps, vel_east_mps, vel_down_mps] = telemetry.velocity_ned_mps;
        let [w, x, y, z] = telemetry.quaternion_wxyz;
        self.flight.observe(VehicleState {
            north_m,
            east_m,
            vel_north_mps,
            vel_east_mps,
            height_m: Some(-down_m),
            climb_mps: -vel_down_mps,
            yaw_rad: yaw_of_quaternion(w, x, y, z),
            armed: match telemetry.arm_state {
                ARM_STATE_ARMED => Some(true),
                ARM_STATE_DISARMED => Some(false),
                _ => None,
            },
        });
    }

    /// One control tick. Before the first telemetry sample the demand is
    /// neutral, because the lease needs frames.
    pub fn step(&mut self, now_s: f64) -> Tick {
        match self.flight.step(now_s) {
            Some(step) => Tick {
                demand: step.demand,
                arm: step.action == Some(Discrete::Arm),
                disarm: step.action == Some(Discrete::Disarm),
                phase: step.phase,
            },
            None => Tick {
                demand: Demand::default(),
                arm: false,
                disarm: false,
                phase: self.flight.phase(),
            },
        }
    }

    /// Gives the executor the host's answer to an arm or a disarm request.
    pub fn on_action_result(&mut self, arm: bool, accepted: bool) {
        let request = if arm { Discrete::Arm } else { Discrete::Disarm };
        self.flight.on_action_result(request, accepted);
    }

    /// Sends the vehicle home to land. The operator's release of the agent in
    /// the air uses this, so no model and no message take part.
    pub fn return_to_base(&mut self, now_s: f64) -> bool {
        self.flight.fly(&Directive::ReturnToBase {}, now_s).is_ok()
    }

    /// The executor phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.flight.phase()
    }

    /// True when the vehicle is on the ground with nothing to fly.
    #[must_use]
    pub const fn is_down(&self) -> bool {
        matches!(self.flight.phase(), Phase::Idle | Phase::Landed)
    }
}

#[cfg(test)]
mod tests;
