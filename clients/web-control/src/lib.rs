//! Rust/WASM web-control runtime for the browser viewer (ADR-0007).
//!
//! The browser owns only Gamepad API sampling, DOM, WebTransport, and the
//! execution of returned actions. This crate owns everything else: device
//! identity resolution and per-device routing (the [`device`] stage, driven
//! by shared-engine profile data), deadzone/expo/inversion, the gimbal
//! quasimode (modifier capture, flight-input masking, R3 edge detection,
//! entry/exit neutralization), lease planning, and the runtime state —
//! behind one [`ControlRuntime::evaluate`] call per control tick.
//!
//! Mappings enter through one validated seam: [`ProfileRuntime::compile`]
//! turns candidate bytes (built-in, imported, cached, or restored — the core
//! cannot tell) into a [`CompiledProfile`], and [`ControlRuntime::activate`]
//! swaps it in through a neutral, transactional handover. There is no
//! privileged default path.

mod device;
mod flight;
mod plan;
mod profile;
mod quasimode;
mod runtime;
mod sample;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(test)]
mod golden;

pub use device::{DeviceStage, MAX_AXES, MAX_BUTTONS, SelectOutcome, parse_gamepad_identity};
pub use plan::{
    AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ActivationPlan, BUTTON_EDGE_PRESSED,
    ControlPlan, Frame, GIMBAL_NEUTRAL_BUTTON, GIMBAL_SCOPE, LeaseAction, MOTION_SCOPE,
};
pub use profile::{
    CompiledProfile, DEFAULT_PROFILE_BYTES, ProfileError, ProfileRuntime, SCHEMA_VERSION,
};
pub use runtime::ControlRuntime;
pub use sample::{ButtonSample, Mode, RawSample, SessionState};
