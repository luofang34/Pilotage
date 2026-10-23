//! The decision port (ADR-0052, ADR-0042): typed judgments over a state
//! and images. A judgment is a choice among named options, a yes/no
//! probability, or a score on ordered levels, each answered with
//! probabilities. The port is the boundary between Pilotage and a judgment
//! service; Pilotage links no model.
//!
//! The request and answer shapes follow the System One decision API that
//! `llama-server` serves on `/v1/systemone` and that hosted decision
//! services serve. A [`Provider`] adapts the shared shapes to one service:
//! its endpoint, its authentication and its quirks, such as text
//! normalization or a yes/no floor. A judgment informs a lane or a planner;
//! it never actuates a vehicle.

mod answer;
#[cfg(feature = "http")]
mod client;
mod error;
mod provider;
mod request;
pub mod survey;

#[cfg(test)]
mod tests;

pub use answer::{Answer, DecisionResponse};
#[cfg(feature = "http")]
pub use client::DecisionClient;
pub use error::DecisionError;
pub use provider::{HttpCall, Provider};
pub use request::{DecisionRequest, Image, Question};
