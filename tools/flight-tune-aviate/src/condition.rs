//! Launching and verifying one deterministic uncertainty condition.
//!
//! A non-nominal run states its uncertainty once, as one content-addressed
//! artifact and the identities that name it. The executor returns those
//! identities before it arms, then states every sample it flew. Nothing here
//! trusts a summary: each sample is answered only after the decision it
//! states has been derived again from the declaration.
//!
//! SIM / NOT FOR FLIGHT.

mod error;
mod frame;
mod launch;
mod projection;
pub mod protocol;
mod session;

pub use error::AviateConditionError;
pub use frame::MAX_FRAME_BYTES;
pub use launch::{ConditionLaunch, TUNING_TRACE_SCHEMA_VERSION};
pub use session::{ConditionTracePath, accept_timeout};

#[cfg(test)]
mod tests;
