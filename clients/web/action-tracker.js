// Reliable delivery for typed discrete actions (CTRL-01): control frames
// ride droppable datagrams, so each press takes a correlation id and rides
// EVERY outgoing frame for its scope until the host's ControlActionResult
// echoes the id (the host deduplicates repeats, so the vehicle executes the
// press exactly once) or the retry window expires. Local mode state changes
// only on acceptance, never optimistically. Pure state-transition helpers,
// unit tested off the DOM.

/** How long a press keeps retransmitting before the client gives up and
 * reports the failure. Generous against RTT + one lease hiccup, short
 * enough that a dead scope surfaces within two seconds. */
export const ACTION_TIMEOUT_MS = 1500;

/** Fresh tracker: no pending presses, ids start at 1 (0 on the wire means
 * "no correlation"). */
export function createActionTracker() {
  return { pending: new Map(), nextId: 1 };
}

function takeId(tracker) {
  const id = tracker.nextId;
  // Wraps within u32, skipping 0.
  tracker.nextId = (tracker.nextId % 0xffffffff) + 1;
  return id;
}

/**
 * Enqueues a press for reliable delivery. `cancels` lists action codes the
 * new press supersedes (arm cancels a pending disarm and vice versa — the
 * host rejects a frame carrying both; a new mode request supersedes any
 * pending one). A press identical to one already pending keeps the original
 * id: the host may already have executed it, and a fresh id would execute
 * it twice.
 */
export function enqueueAction(tracker, scope, action, nowMs, options = {}) {
  const { modeTarget, cancels = [] } = options;
  for (const [id, entry] of tracker.pending) {
    if (entry.scope !== scope) continue;
    if (cancels.includes(entry.action)) {
      tracker.pending.delete(id);
      continue;
    }
    if (entry.action === action && entry.modeTarget === modeTarget) {
      return id;
    }
  }
  const id = takeId(tracker);
  tracker.pending.set(id, { scope, action, modeTarget, enqueuedAtMs: nowMs });
  return id;
}

/**
 * The actions to attach to the next outgoing frame for `scope` — every
 * still-pending press, each carrying its correlation id — plus the presses
 * whose retry window expired this tick (removed; the caller reports them).
 */
export function frameActions(tracker, scope, nowMs) {
  const actions = [];
  const expired = [];
  for (const [id, entry] of tracker.pending) {
    if (entry.scope !== scope) continue;
    if (nowMs - entry.enqueuedAtMs > ACTION_TIMEOUT_MS) {
      tracker.pending.delete(id);
      expired.push({ ...entry, actionId: id });
      continue;
    }
    actions.push(
      entry.modeTarget === undefined
        ? { action: entry.action, actionId: id }
        : { action: entry.action, modeTarget: entry.modeTarget, actionId: id },
    );
  }
  return { actions, expired };
}

/**
 * Settles the press a `ControlActionResult` answers, returning it (or null
 * for an unknown id — a replay of an already-settled press, harmless). The
 * caller applies mode changes only from a returned, accepted entry.
 */
export function resolveAction(tracker, actionId) {
  const entry = tracker.pending.get(actionId) ?? null;
  if (entry) tracker.pending.delete(actionId);
  return entry;
}

/** The mode target of a pending mode request on `scope`, if any — the base
 * a repeated toggle press flips from before the first ack lands. */
export function pendingModeTarget(tracker, scope, modeRequestCode) {
  for (const entry of tracker.pending.values()) {
    if (entry.scope === scope && entry.action === modeRequestCode) {
      return entry.modeTarget;
    }
  }
  return undefined;
}
