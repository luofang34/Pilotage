//! Tracks connected clients' outbound channels so the engine actor can
//! unicast or broadcast [`SessionAction`]s without knowing about transport
//! internals (ADR-0005).
//!
//! [`SessionAction`]: pilotage_session::SessionAction

use std::collections::BTreeMap;

use pilotage_mission::Requester;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;

use crate::runtime::connection::ToConnection;

/// Bound of the per-client outbound queue.
///
/// A connection's writer half drains this as fast as the transport allows;
/// generous headroom absorbs a burst of authority broadcasts without
/// dropping, while still bounding memory if a client's write side stalls.
pub const OUTBOUND_QUEUE_CAPACITY: usize = 256;

/// Maps live [`ClientKey`]s to the channel that delivers messages to their
/// connection task.
#[derive(Debug, Default)]
pub struct ClientRegistry {
    clients: BTreeMap<ClientKey, mpsc::Sender<ToConnection>>,
    /// Who each admitted connection is to the mission lane.
    requesters: BTreeMap<ClientKey, Requester>,
}

impl ClientRegistry {
    /// Constructs an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a newly accepted connection's outbound sender.
    pub fn insert(&mut self, client: ClientKey, sender: mpsc::Sender<ToConnection>) {
        self.clients.insert(client, sender);
    }

    /// Removes a client on disconnect.
    pub fn remove(&mut self, client: ClientKey) {
        self.clients.remove(&client);
        self.requesters.remove(&client);
    }

    /// Records who an admitted connection is.
    pub fn set_requester(&mut self, client: ClientKey, requester: Requester) {
        self.requesters.insert(client, requester);
    }

    /// Who `client` is to the mission lane. A client that no connection
    /// admitted runs inside the host, so it may approve.
    #[must_use]
    pub fn requester(&self, client: ClientKey) -> Requester {
        self.requesters
            .get(&client)
            .cloned()
            .unwrap_or_else(|| Requester {
                subject: format!("in-process-{}", client.as_u64()),
                may_approve: true,
            })
    }

    /// Returns the sender for one client, if it is still connected.
    #[must_use]
    pub fn sender(&self, client: ClientKey) -> Option<&mpsc::Sender<ToConnection>> {
        self.clients.get(&client)
    }

    /// Iterates every currently connected client's key and sender, for
    /// broadcast fan-out.
    pub fn iter(&self) -> impl Iterator<Item = (ClientKey, &mpsc::Sender<ToConnection>)> {
        self.clients.iter().map(|(key, sender)| (*key, sender))
    }
}
