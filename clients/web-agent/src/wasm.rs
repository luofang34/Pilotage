//! The wasm-bindgen surface of the agent module.
//!
//! Structured values cross as JSON text: the model request, the model reply
//! and the decision. The shell moves them between this module and the model
//! gateway and does not read a directive. The control tick crosses as
//! numbers, because it runs at the frame rate.

use wasm_bindgen::prelude::*;

use crate::module::{AGENT_PROFILE_ID, AgentModule, Telemetry};

/// Layout of the tick result: roll, pitch, throttle, yaw, then the arm and
/// disarm requests as `0.0` or `1.0`.
const TICK_LEN: usize = 6;

/// The JS-owned agent resource.
#[wasm_bindgen]
pub struct WebAgent {
    module: AgentModule,
}

#[wasm_bindgen]
impl WebAgent {
    /// Starts a flight on the chart document, under what the motion scope
    /// advertises.
    #[wasm_bindgen(constructor)]
    pub fn new(
        chart_json: &str,
        max_linear_mps: f64,
        disarm_offered: bool,
    ) -> Result<WebAgent, JsError> {
        AgentModule::new(chart_json, max_linear_mps, disarm_offered)
            .map(|module| Self { module })
            .map_err(|error| JsError::new(&describe(&error)))
    }

    /// The identity that the control runtime announces for this agent.
    #[must_use]
    pub fn profile_id() -> String {
        AGENT_PROFILE_ID.to_owned()
    }

    /// The model request for one operator message, as one JSON line.
    pub fn request(&self, message: &str) -> Result<String, JsError> {
        self.module
            .request_json(message)
            .map_err(|error| JsError::new(&describe(&error)))
    }

    /// Takes the reply line of the model gateway. Returns the decision as
    /// JSON with a `result` of `flown`, `refused` or `fault`.
    pub fn take_reply(&mut self, message: &str, reply_json: &str, now_s: f64) -> String {
        let decision = self.module.take_reply_json(message, reply_json, now_s);
        serde_json::to_string(&decision).unwrap_or_else(|_| {
            r#"{"result":"fault","detail":"the decision did not encode"}"#.to_owned()
        })
    }

    /// Records one operational-estimate sample in local NED.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        position_ned_m: &[f64],
        velocity_ned_mps: &[f64],
        quaternion_wxyz: &[f64],
        arm_state: u32,
    ) -> bool {
        let (Ok(position_ned_m), Ok(velocity_ned_mps), Ok(quaternion_wxyz)) = (
            position_ned_m.try_into(),
            velocity_ned_mps.try_into(),
            quaternion_wxyz.try_into(),
        ) else {
            return false;
        };
        self.module.observe(Telemetry {
            position_ned_m,
            velocity_ned_mps,
            quaternion_wxyz,
            arm_state,
        });
        true
    }

    /// One control tick: roll, pitch, throttle, yaw, arm request, disarm
    /// request.
    pub fn step(&mut self, now_s: f64) -> Vec<f32> {
        let tick = self.module.step(now_s);
        let flag = |set: bool| if set { 1.0 } else { 0.0 };
        let mut out = Vec::with_capacity(TICK_LEN);
        out.extend([
            tick.demand.roll,
            tick.demand.pitch,
            tick.demand.throttle,
            tick.demand.yaw,
            flag(tick.arm),
            flag(tick.disarm),
        ]);
        out
    }

    /// Gives the executor the host's answer to an arm or a disarm request.
    pub fn on_action_result(&mut self, arm: bool, accepted: bool) {
        self.module.on_action_result(arm, accepted);
    }

    /// Sends the vehicle home to land.
    pub fn return_to_base(&mut self, now_s: f64) -> bool {
        self.module.return_to_base(now_s)
    }

    /// The executor phase as its wire name, such as `enroute`.
    #[must_use]
    pub fn phase(&self) -> String {
        serde_json::to_value(self.module.phase())
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// True when the vehicle is on the ground with nothing to fly.
    #[must_use]
    pub fn is_down(&self) -> bool {
        self.module.is_down()
    }
}

/// An error with each cause, because a `JsError` carries one string.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
