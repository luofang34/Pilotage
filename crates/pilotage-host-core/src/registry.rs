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
    /// The messages still owed to clients the engine removed, such as the
    /// `Close` a removal itself queues: a removed client still reads what
    /// was queued before it was removed, once.
    departed: BTreeMap<ClientKey, VecDeque<ToConnection>>,
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

    /// Removes a client; the messages already queued for it stay owed to
    /// it until they are taken.
    pub(crate) fn remove(&mut self, client: ClientKey) {
        if let Some(queue) = self.clients.remove(&client)
            && !queue.is_empty()
        {
            self.departed.entry(client).or_default().extend(queue);
        }
    }

    /// Forgets a client whose connection is gone, with anything still owed
    /// to it.
    pub(crate) fn forget(&mut self, client: ClientKey) {
        self.clients.remove(&client);
        self.departed.remove(&client);
    }

    /// The clients with messages to read, connected or departed, in key
    /// order.
    pub(crate) fn keys(&self) -> Vec<ClientKey> {
        let mut keys: Vec<ClientKey> = self
            .clients
            .keys()
            .chain(self.departed.keys())
            .copied()
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
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
        let mut messages = self.departed.remove(&client).unwrap_or_default();
        if let Some(queue) = self.clients.get_mut(&client) {
            messages.extend(core::mem::take(queue));
        }
        messages
    }
}
