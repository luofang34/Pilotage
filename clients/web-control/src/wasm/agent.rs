//! The wasm surface of the automation input source (ADR-0042).

use wasm_bindgen::prelude::*;

use super::{FLAG_AGENT_OVERRIDE, WebControl};
use crate::coordinator::{AgentInput, AgentTick};
use crate::sample::{ButtonSample, DirectDemand};

impl WebControl {
    /// Refills the sample for one agent tick from the stored agent input and
    /// the operator's pad, and returns the flag for an operator override.
    pub(super) fn load_agent_sample(&mut self, axes: &[f32], buttons: &[ButtonSample]) -> u32 {
        let input = core::mem::take(&mut self.agent_input);
        let tick = self
            .coordinator
            .agent_sample(axes, buttons, input, &mut self.sample);
        if tick == AgentTick::OperatorOverride {
            FLAG_AGENT_OVERRIDE
        } else {
            0
        }
    }
}

#[wasm_bindgen]
impl WebControl {
    /// Engages an agent as the input source under the identity that it
    /// announces. Returns `false` when the runtime refuses.
    pub fn engage_agent(&mut self, profile_id: &str) -> bool {
        self.coordinator.engage_agent(profile_id)
    }

    /// Returns control to the operator's devices.
    pub fn disengage_agent(&mut self) {
        self.coordinator.disengage_agent();
    }

    /// Whether an agent is the input source, or will be when the open
    /// handover installs.
    #[must_use]
    pub fn agent_engaged(&self) -> bool {
        self.coordinator.agent_engaged()
    }

    /// Sets the agent's input for the next agent tick. One tick consumes it:
    /// a tick with no new input flies neutral.
    #[allow(clippy::too_many_arguments)]
    pub fn set_agent_demand(
        &mut self,
        roll: f32,
        pitch: f32,
        throttle: f32,
        yaw: f32,
        arm: bool,
        disarm: bool,
    ) {
        self.agent_input = AgentInput {
            demand: DirectDemand {
                roll,
                pitch,
                throttle,
                yaw,
            },
            arm,
            disarm,
        };
    }
}
