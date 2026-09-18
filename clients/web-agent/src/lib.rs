//! The web port of the agent client module (ADR-0042).
//!
//! The browser shell owns the operator text box, the connection to a model
//! gateway, and the telemetry that it already decodes for the instruments.
//! Each decision stays in shared Rust: this crate wraps
//! [`pilotage_agent::AgentFlight`], so the browser flies the same checks and
//! the same executor as the headless port. The demand that this module gives
//! enters control through the automation input source of
//! `pilotage-control-web`. This crate sends nothing.

mod error;
mod module;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub use error::ModuleError;
pub use module::{AGENT_PROFILE_ID, AgentModule, Decision, Telemetry, Tick};
