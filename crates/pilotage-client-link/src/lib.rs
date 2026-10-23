//! The native WebTransport port of the client session
//! (`pilotage-client-session`): it connects to a session host, pins the
//! host's certificate hash, opens the reliable bootstrap stream, and turns
//! every byte the host sends into a [`TransportEvent`]. The client engine
//! decides what the bytes mean. There is no option that turns certificate
//! validation off.

mod error;
mod link;

pub use error::LinkError;
pub use link::Link;
