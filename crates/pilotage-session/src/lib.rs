//! Host-side session state machine for Pilotage (ADR-0005, ADR-0006,
//! ADR-0009, ADR-0010).
//!
//! [`SessionEngine`] is a pure, sans-IO state machine (ADR-0002) the host
//! binary drives: it consumes decoded [`DomainEnvelope`]s plus an explicit
//! `now` and returns [`SessionAction`]s the driver executes against the
//! WebTransport session and the adapter. Handshake, lease grant, per-frame
//! fencing, staleness rejection, RTT echo, and disconnect release all live
//! here, so the host binary carries no protocol decision logic.
//!
//! Authority is delegated to an embedded [`pilotage_authority::AuthorityEngine`]:
//! lease grants and offer expiry route through it, and its effects become
//! [`SessionAction::Broadcast`]s on the ordered authority stream.
//!
//! The staleness check is a documented loopback-only simplification for
//! increment 0; see [`SessionEngine`] for the full caveat and the RTT/offset
//! follow-up.

mod action;
mod capabilities;
mod clients;
mod command_gate;
mod config;
mod engine;
mod liveness;
mod message;
mod outbound;

#[cfg(test)]
mod tests;

pub use action::{CloseReason, LinkLossTrigger, SessionAction, SessionOutcome};
pub use command_gate::gate_frame;
pub use config::SessionConfig;
pub use engine::SessionEngine;
pub use message::{ClientKey, DomainEnvelope};
pub use outbound::OutboundMessage;
