//! Portable sans-IO client-session core for operator clients (ADR-0037).
//!
//! [`ClientEngine`] is the one state machine every operator client drives,
//! whatever its platform: a transport port feeds it [`TransportEvent`]s plus
//! an explicit monotonic `now`, and it returns [`ClientAction`]s the port
//! executes against the socket. Bootstrap, admission, stream classification,
//! the authority mirror, control fencing, and reconnect decisions all live
//! here, so neither JavaScript nor Swift decodes a Pilotage message or
//! derives canonical session state.
//!
//! The core is read-only by default. Control is an explicit request from the
//! shell ([`ClientEngine::request_lease`]), and a reconnect restores
//! observation only: authority does not survive a transport loss, so a lease
//! is never requested during recovery (ADR-0037).

mod action;
mod authority;
mod bootstrap;
mod catalog;
mod control;
mod engine;
mod event;
mod motion;
mod reconnect;
mod streams;

#[cfg(test)]
mod tests;

pub use action::{ClientAction, ClientFault, ModuleEvent};
pub use authority::AuthorityMirror;
pub use catalog::{Admission, ScopeCatalog, VehicleCatalog};
pub use control::{ControlCommand, ControlLane};
pub use engine::{ClientConfig, ClientEngine, ClientPhase};
pub use event::{StreamId, TransportEvent};
pub use motion::{MotionDemand, intent_capability, velocity_intent};
pub use reconnect::ReconnectPolicy;
