//! MAVLink 2.0 subset codec and receive-link machinery shared by
//! flight-controller adapters (ADR-0018).
//!
//! The [`codec`] module is pure byte functions — framing, CRC-EXTRA,
//! decoding of the standard message subset (heartbeat, attitude,
//! local position, command ack, estimator status) plus the Aviate
//! private estimator-status extension — unit-testable byte-for-byte.
//! The [`link`] module owns the UDP receive task and the shared
//! latest-state cache with the acquisition-clock discipline: source
//! epochs, reboot detection, inter-group skew budgets, and staleness
//! stamps. Adapters sample the cache; they never touch the socket.

pub mod codec;
pub mod link;

pub use codec::{FcMessage, FrameSource, ParseStats, parse_datagram};
pub use link::{
    AttitudeUpdate, KinematicsUpdate, LinkConfig, LinkError, LinkState, MavlinkLink, ResetPolicy,
};
