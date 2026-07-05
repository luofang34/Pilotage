//! Lease, generation, handover, override, and link-loss state machines for
//! capability-scoped control authority (ADR-0006, ADR-0010).
//!
//! This crate is sans-IO: state machines are driven by explicit commands and
//! `now` timestamps supplied by the embedding session host, per ADR-0002. The
//! central type is [`AuthorityEngine`]; it maintains per-`(vehicle, scope)`
//! authority and turns [`AuthorityCommand`]s into ordered [`AuthorityEffect`]s.
//!
//! Effective authority for a normal handover changes at exactly one atomic
//! point — the engine committing an [`AuthorityCommand::Accept`] — after which
//! the previous holder's frames are fenced out by generation (ADR-0010).
//! Confirmations never gate a transfer. Offers expire after a caller-supplied
//! TTL back to the offerer.

mod command;
mod effect;
mod engine;
mod state;
mod wire;

#[cfg(test)]
mod tests;

pub use command::{AuthorityClass, AuthorityCommand, LinkState, OverrideReason};
pub use effect::{AuthorityEffect, AuthorityWarning, FrameVerdict, RejectReason};
pub use engine::AuthorityEngine;
pub use wire::WireEventKind;
