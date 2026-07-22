// Pending-press bookkeeping for typed discrete actions (CTRL-01): each
// press takes a nonzero correlation id and is sent ONCE on the reliable
// ordered session stream (`ControlActionCommand`); the entry stays pending
// until the host's ControlActionResult echoes the id or the answer window
// expires (transport death — the reliable channel otherwise always
// answers). Local mode state changes only on acceptance, never
// optimistically. Pure state-transition helpers, unit tested off the DOM.

/** How long a press may stay unanswered before the client reports the
 * failure. The reliable stream always answers unless the transport died;
 * generous against RTT, short enough that a dead session surfaces within
 * two seconds. */
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
 * Removes and returns every pending press (any scope) whose answer window
 * expired — the caller reports each loudly. On the reliable channel an
 * expiry means the transport died or the host never answered, never a
 * dropped datagram.
 */
export function expirePending(tracker, nowMs) {
  const expired = [];
  for (const [id, entry] of tracker.pending) {
    if (nowMs - entry.enqueuedAtMs > ACTION_TIMEOUT_MS) {
      tracker.pending.delete(id);
      expired.push({ ...entry, actionId: id });
    }
  }
  return expired;
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
