//! The shared telemetry-to-instrument feed (ADR-0037).
//!
//! Between the wire and the instrument runtime stands one set of
//! decisions: which publications the ingress admits, how a heading is
//! declared, what the turn derivation accepts, and how the admitted lanes
//! assemble into the state frame the panels read. The browser makes these
//! decisions in viewer script today; a second client must not make them a
//! second time. [`InstrumentFeed`] is that one implementation: wire
//! telemetry samples in, encoded v7 state frames out, every judgement
//! delegated to the shared feeder.
//!
//! The feed depends on the protocol AND the runtime, like the viewer glue
//! it replaces; the ADR-0034 cut stays intact because each side remains
//! its own crate and this one holds only the joining.

mod sample;
mod state;

#[cfg(test)]
mod tests;

pub use sample::{avionics_sample, raw_stamp};
pub use state::{FeedParams, InstrumentFeed};
