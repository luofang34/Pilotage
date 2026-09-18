//! Errors of the web agent module.

use pilotage_agent::AgentError;

/// Why the module cannot start or cannot build a request.
#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    /// The chart document is not a valid scenario.
    #[error("the agent chart is not usable")]
    Chart(#[source] AgentError),
    /// The vehicle advertises no usable speed.
    #[error("the advertised speed limit {max_linear_mps} m/s is not usable")]
    SpeedLimit {
        /// The advertised speed at full demand.
        max_linear_mps: f64,
    },
    /// A model request did not encode.
    #[error("cannot encode the model request")]
    EncodeRequest(#[source] serde_json::Error),
}
