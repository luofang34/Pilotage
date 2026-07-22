//! Per-client exactly-once bookkeeping for correlated discrete actions
//! (CTRL-01 reliable delivery): control frames ride droppable datagrams, so
//! a sender retransmits an action — same correlation id — on successive
//! frames until the matching `ControlActionResult` echoes the id. This cache
//! deduplicates those repeats so the vehicle executes each press exactly
//! once, and replays the recorded result so every retransmission still gets
//! its answer.

use std::collections::{HashMap, VecDeque};

use pilotage_protocol::{ControlActionResult, ScopedControlFrame};
use pilotage_session::ClientKey;

/// Answered correlation ids retained per client. Far above any client's
/// in-flight action window (a handful); eviction below this bound would
/// re-execute a very stale retransmission.
const RESULTS_PER_CLIENT: usize = 64;

#[derive(Default)]
struct ClientActions {
    results: HashMap<u32, ControlActionResult>,
    /// Insertion order for bounded eviction.
    order: VecDeque<u32>,
}

/// Deduplication state across all connected clients.
#[derive(Default)]
pub(super) struct ActionDedup {
    per_client: HashMap<ClientKey, ClientActions>,
}

impl ActionDedup {
    /// Removes already-answered actions from `frame`, returning the cached
    /// results to re-send for them. Uncorrelated actions (id 0, legacy
    /// translation) always pass through — they carry no identity to
    /// deduplicate on.
    pub(super) fn strip_answered(
        &self,
        client: ClientKey,
        frame: &mut ScopedControlFrame,
    ) -> Vec<ControlActionResult> {
        let Some(state) = self.per_client.get(&client) else {
            return Vec::new();
        };
        let mut replays = Vec::new();
        let mut kept_actions = Vec::with_capacity(frame.actions.len());
        let mut kept_ids = Vec::with_capacity(frame.actions.len());
        for (index, action) in frame.actions.iter().enumerate() {
            let id = frame.action_ids.get(index).copied().unwrap_or(0);
            match state.results.get(&id) {
                Some(cached) if id != 0 => replays.push(cached.clone()),
                _ => {
                    kept_actions.push(*action);
                    kept_ids.push(id);
                }
            }
        }
        frame.actions = kept_actions;
        frame.action_ids = kept_ids;
        replays
    }

    /// Records the result answering a correlated action so later
    /// retransmissions replay it instead of re-executing.
    pub(super) fn record(&mut self, client: ClientKey, result: ControlActionResult) {
        if result.action_id == 0 {
            return;
        }
        let state = self.per_client.entry(client).or_default();
        if state
            .results
            .insert(result.action_id, result.clone())
            .is_none()
        {
            state.order.push_back(result.action_id);
            while state.order.len() > RESULTS_PER_CLIENT {
                if let Some(evicted) = state.order.pop_front() {
                    state.results.remove(&evicted);
                }
            }
        }
    }

    /// Drops a disconnected client's cache; its ids are meaningless to any
    /// later connection.
    pub(super) fn forget(&mut self, client: ClientKey) {
        self.per_client.remove(&client);
    }
}
