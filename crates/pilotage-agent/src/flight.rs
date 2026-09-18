//! One agent flight, as each platform port drives it.
//!
//! A port owns the transport, the clock and the model connection. This type
//! owns all else: the envelope and the legend for a model request, the three
//! checks between a model reply and the vehicle, the executor, and the newest
//! vehicle state. The headless port and a client shell then cannot differ in
//! what they fly.

use crate::capability::{FlightEnvelope, NumberRange};
use crate::directive::Directive;
use crate::executor::{Discrete, Executor, FlightLimits, Phase, Step};
use crate::grounding::check_grounding;
use crate::model_port::{Frame, ModelReply, ModelRequest, Refusal, check_reply};
use crate::scenario::Scenario;
use crate::state::VehicleState;

/// Share of the advertised speed limit used as the cruise speed.
const CRUISE_SHARE: f64 = 0.8;
/// The slowest speed that a `speed` directive can ask for.
const MIN_SPEED_MPS: f64 = 0.3;

/// What the motion scope of the vehicle advertises to the agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleOffer {
    /// Advertised speed at full horizontal demand, in metres per second.
    pub max_linear_mps: f64,
    /// True when the motion scope advertises a disarm.
    pub disarm_offered: bool,
}

/// One agent flight.
#[derive(Debug)]
pub struct AgentFlight {
    envelope: FlightEnvelope,
    legend: String,
    executor: Executor,
    state: Option<VehicleState>,
}

impl AgentFlight {
    /// A flight on the ground, with the chart of `scenario` and the limits
    /// that the vehicle advertises.
    #[must_use]
    pub fn new(scenario: &Scenario, offer: VehicleOffer) -> Self {
        let limits = FlightLimits {
            cruise_height_m: scenario.cruise_height_m,
            cruise_speed_mps: CRUISE_SHARE * offer.max_linear_mps,
            arrival_radius_m: scenario.arrival_radius_m,
            max_range_m: scenario.max_range_m,
            max_linear_mps: offer.max_linear_mps,
            disarm_offered: offer.disarm_offered,
        };
        Self {
            envelope: scenario.envelope(NumberRange {
                min: MIN_SPEED_MPS.min(offer.max_linear_mps),
                max: offer.max_linear_mps,
            }),
            legend: scenario.legend(),
            executor: Executor::new(limits, scenario),
            state: None,
        }
    }

    /// What the agent can fly now.
    #[must_use]
    pub const fn envelope(&self) -> &FlightEnvelope {
        &self.envelope
    }

    /// The model request for one operator message.
    #[must_use]
    pub fn request(&self, message: &str, frames: Vec<Frame>) -> ModelRequest {
        ModelRequest {
            id: 0,
            message: message.to_owned(),
            envelope: self.envelope.clone(),
            legend: self.legend.clone(),
            frames,
        }
    }

    /// Takes the reply of a model to `message`. Three checks stand between a
    /// reply and the vehicle: the envelope, the words of the message, and the
    /// chart of the executor. A refused reply changes nothing.
    pub fn take_reply(
        &mut self,
        message: &str,
        reply: &ModelReply,
        now_s: f64,
    ) -> Result<Directive, Refusal> {
        let directive = check_reply(&self.envelope, reply)?;
        check_grounding(message, &directive)?;
        self.fly(&directive, now_s)?;
        Ok(directive)
    }

    /// Flies a directive that deterministic code gives, such as the return to
    /// base at the end of a run. No model and no operator message take part,
    /// so only the chart check applies.
    pub fn fly(&mut self, directive: &Directive, now_s: f64) -> Result<(), Refusal> {
        self.executor.accept(directive, self.state.as_ref(), now_s)
    }

    /// Records the newest vehicle state.
    pub fn observe(&mut self, state: VehicleState) {
        self.state = Some(state);
    }

    /// The newest vehicle state, when there is one.
    #[must_use]
    pub const fn state(&self) -> Option<&VehicleState> {
        self.state.as_ref()
    }

    /// One executor step on the newest state. `None` before the first state:
    /// the port then sends a neutral demand, because the lease needs frames.
    pub fn step(&mut self, now_s: f64) -> Option<Step> {
        let state = self.state?;
        Some(self.executor.step(&state, now_s))
    }

    /// Gives the executor the host's answer to a discrete request.
    pub fn on_action_result(&mut self, request: Discrete, accepted: bool) {
        self.executor.on_action_result(request, accepted);
    }

    /// The executor phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.executor.phase()
    }

    /// Seconds in the air, for a scripted message that waits for flight.
    #[must_use]
    pub fn flying_seconds(&self, now_s: f64) -> Option<f64> {
        self.executor.flying_seconds(now_s)
    }
}

#[cfg(test)]
mod tests;
