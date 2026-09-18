//! The headless port of the agent client module.
//!
//! `pilotage-agent` is the shared core with no I/O. This crate gives it a
//! platform: a process port for a model adapter, a WebTransport session with
//! the host, and the files of a run. Two programs use it. `intent-pilot` flies
//! a scenario or live operator messages. `agent-eval` scores a model adapter
//! against a case suite.

mod model_process;

pub use model_process::{ModelProcess, ModelProcessError};
