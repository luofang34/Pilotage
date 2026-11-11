//! The engine's in-core clients: the messages queued for each, in order.
//! A process host writes them to transports; the core's clients read
//! them on the same call.

use std::collections::{BTreeMap, VecDeque};

use pilotage_session::ClientKey;

use crate::connection::ToConnection;

/// The queue of each connected in-core client.
#[derive(Debug, Default)]
pub(crate) struct ClientRegistry {
    clients: BTreeMap<ClientKey, VecDeque<ToConnection>>,
}

impl ClientRegistry {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registers a connected client.
    pub(crate) fn insert(&mut self, client: ClientKey) {
        self.clients.entry(client).or_default();
    }

    /// Removes a client on disconnect.
    pub(crate) fn remove(&mut self, client: ClientKey) {
        self.clients.remove(&client);
    }

    /// The connected clients, in key order.
    pub(crate) fn keys(&self) -> Vec<ClientKey> {
        self.clients.keys().copied().collect()
    }

    /// Queues `message` for one client; false when it is not connected.
    pub(crate) fn send(&mut self, client: ClientKey, message: ToConnection) -> bool {
        match self.clients.get_mut(&client) {
            Some(queue) => {
                queue.push_back(message);
                true
            }
            None => false,
        }
    }

    /// Queues `message` for every connected client.
    pub(crate) fn broadcast(&mut self, message: &ToConnection) {
        for queue in self.clients.values_mut() {
            queue.push_back(message.clone());
        }
    }

    /// Takes the messages queued for one client, in order.
    pub(crate) fn take(&mut self, client: ClientKey) -> VecDeque<ToConnection> {
        self.clients
            .get_mut(&client)
            .map(core::mem::take)
            .unwrap_or_default()
    }
}
