//! Portable operator control runtime (ADR-0007), shared by every operator
//! client: the browser viewer reaches it through `pilotage-control-web`'s
//! wasm-bindgen surface, and native clients link it directly, so one
//! mapping serves both and the two cannot drift apart.
//!
//! A shell owns only device sampling, its UI, the transport, and the
//! execution of returned actions. This crate owns everything else: device
//! identity resolution and per-device routing (the device stage, driven
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

mod authority;
mod coordinator;
mod device;
mod flight;
mod plan;
mod profile;
mod quasimode;
mod runtime;
mod sample;
mod telegraph;
mod transition;

#[cfg(test)]
mod golden;

pub use authority::{
    AuthorityDisposition, AuthorityEvent, AuthorityScope, AuthorityState, is_fresh_generation,
};
pub use coordinator::{AgentInput, AgentTick, ControlCoordinator, InputSource};
pub use device::{DeviceStage, MAX_AXES, MAX_BUTTONS, SelectOutcome, parse_gamepad_identity};
pub use plan::{
    AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ActivationPlan, BUTTON_EDGE_PRESSED,
    ControlPlan, Frame, GIMBAL_NEUTRAL_BUTTON, GIMBAL_SCOPE, LeaseAction, MOTION_SCOPE,
};
pub use profile::{
    CompiledProfile, DEFAULT_PROFILE_BYTES, ProfileError, ProfileRuntime, SCHEMA_VERSION,
};
pub use runtime::ControlRuntime;
pub use sample::{ButtonSample, DirectDemand, Mode, RawSample, SessionState};
pub use telegraph::{ArmConfirmed, ArmOrder, ArmTelegraph, OrderAction, TelegraphPhase};
pub use transition::{AuthorityRoute, DIRECT_SCOPE, LIFECYCLE_SCOPE};
