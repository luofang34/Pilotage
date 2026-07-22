//! Per-client exactly-once and ANTI-REPLAY bookkeeping for correlated
//! discrete actions (CTRL-01 reliable delivery). A bounded cache re-answers
//! true replays; a monotonic watermark makes every id at or below the
//! newest admitted one permanently stale — an id evicted from the bounded
//! cache can be refused but never re-executed. Sound because ids are
//! minted sequentially by one sender on one ORDERED stream: a fresh press
//! always advances (wrapping within u32, never minting 0), so a
//! non-advancing id is a replay or a forgery, never legitimate reordering.

use std::collections::{HashMap, VecDeque};

use pilotage_protocol::{
    ControlAction, ControlActionResult, Generation, ScopeId, ScopedControlFrame, VehicleId,
};
use pilotage_session::ClientKey;

/// The immutable content of a correlated action request. A duplicate id
/// must carry an IDENTICAL fingerprint to be treated as a replay; the same
/// id with different content is a correlation-id reuse and is refused —
/// otherwise a reused id could smuggle a different action behind a cached
/// "accepted".
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ActionFingerprint {
    vehicle: VehicleId,
    scope: ScopeId,
    generation: Generation,
    activation_revision: u32,
    action: ControlAction,
}

impl ActionFingerprint {
    fn of(frame: &ScopedControlFrame, action: ControlAction) -> Self {
        Self {
            vehicle: frame.vehicle,
            scope: frame.scope.clone(),
            generation: frame.generation,
            activation_revision: frame.activation_revision,
            action,
        }
    }
}

/// Answered correlation ids retained per client. Far above any client's
/// in-flight action window (a handful); eviction below this bound would
/// re-execute a very stale retransmission.
const RESULTS_PER_CLIENT: usize = 64;

#[derive(Default)]
struct ClientActions {
    results: HashMap<u32, (ActionFingerprint, ControlActionResult)>,
    /// Insertion order for bounded eviction.
    order: VecDeque<u32>,
    /// The newest ADMITTED correlation id: the anti-replay watermark. Ids
    /// that do not advance past it (wrapping-forward within the half
    /// range) are stale forever, even after their result left the bounded
    /// cache.
    latest_id: Option<u32>,
}

/// Whether `id` is a wrapping-forward advance past `latest` (within the
/// u32 half range, so a wrap from near `u32::MAX` to a small id still
/// advances while anything at or behind the watermark does not).
fn advances(latest: u32, id: u32) -> bool {
    let distance = id.wrapping_sub(latest);
    distance != 0 && distance <= u32::MAX / 2
}

/// Deduplication state across all connected clients.
#[derive(Default)]
pub(super) struct ActionDedup {
    per_client: HashMap<ClientKey, ClientActions>,
}

impl ActionDedup {
    /// Removes already-answered actions from `frame`, returning the cached
    /// result to re-send for each true replay and a REJECTED result for
    /// each correlation-id reuse (same id, different fingerprint).
    /// Uncorrelated actions (id 0, legacy translation) always pass through
    /// — they carry no identity to deduplicate on.
    pub(super) fn strip_answered(
        &self,
        client: ClientKey,
        frame: &mut ScopedControlFrame,
    ) -> Vec<ControlActionResult> {
        let Some(state) = self.per_client.get(&client) else {
            return Vec::new();
        };
        let mut answers = Vec::new();
        let mut kept_actions = Vec::with_capacity(frame.actions.len());
        let mut kept_ids = Vec::with_capacity(frame.actions.len());
        for (index, action) in frame.actions.iter().enumerate() {
            let id = frame.action_ids.get(index).copied().unwrap_or(0);
            let refusal = |detail: &str| ControlActionResult {
                vehicle: frame.vehicle,
                scope: frame.scope.clone(),
                generation: frame.generation,
                sequence: frame.sequence,
                action: *action,
                accepted: false,
                detail: detail.to_owned(),
                action_id: id,
            };
            match state.results.get(&id) {
                Some((fingerprint, cached)) if id != 0 => {
                    // A true replay re-answers; the same id carrying
                    // DIFFERENT content is a correlation-id reuse and is
                    // refused — a reused id must never smuggle a different
                    // action behind a cached "accepted".
                    if *fingerprint == ActionFingerprint::of(frame, *action) {
                        answers.push(cached.clone());
                    } else {
                        answers.push(refusal("correlation id reused with different content"));
                    }
                }
                _ if id != 0 && state.latest_id.is_some_and(|latest| !advances(latest, id)) => {
                    // Behind the watermark and no cached answer: the id was
                    // evicted from the bounded cache, or was never seen on
                    // this ORDERED stream in order. Either way it is stale
                    // — refused, never executed.
                    answers.push(refusal("stale correlation id: already superseded"));
                }
                _ => {
                    kept_actions.push(*action);
                    kept_ids.push(id);
                }
            }
        }
        frame.actions = kept_actions;
        frame.action_ids = kept_ids;
        answers
    }

    /// Records the result answering a correlated action (with the request
    /// frame's fingerprint) so later replays re-answer instead of
    /// re-executing.
    pub(super) fn record(
        &mut self,
        client: ClientKey,
        frame: &ScopedControlFrame,
        result: ControlActionResult,
    ) {
        if result.action_id == 0 {
            return;
        }
        let state = self.per_client.entry(client).or_default();
        let fingerprint = ActionFingerprint::of(frame, result.action);
        // Advance the anti-replay watermark: this id (and everything at or
        // behind it) can never execute again, cache or no cache.
        state.latest_id = Some(match state.latest_id {
            Some(latest) if !advances(latest, result.action_id) => latest,
            _ => result.action_id,
        });
        if state
            .results
            .insert(result.action_id, (fingerprint, result.clone()))
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
