//! `pilotage-session-host`: a tokio + WebTransport binary embedding
//! [`pilotage_session::SessionEngine`] and the reference adapter (ADR-0005).
//!
//! All decision logic (handshake, lease, fencing, staleness, authority) lives
//! in `pilotage-session`; this crate is transport plumbing plus process
//! lifecycle. [`runtime::start`] is the entry point both `main` and the
//! in-process loopback integration test use.

pub mod cli;
pub mod error;
pub mod output;
pub mod runtime;
pub mod tls_identity;
