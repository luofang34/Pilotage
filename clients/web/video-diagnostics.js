// Pure decision logic for the demo viewer's video diagnostics: log a
// per-source conformal-readiness change once (not per frame), and coalesce a
// burst of uni-stream read failures. Kept side-effect-free so it is testable
// apart from the WebTransport plumbing in main.js.

// The per-source readiness state a caller remembers: `undefined` = never
// logged, `null` = last logged ready, a string = last logged not-ready reason.

/// Whether a readiness change is worth logging, and the new state to
/// remember. Returns `null` when nothing changed (the common per-frame
/// case), so a persistent state logs exactly once on each transition.
export function readinessTransition(previousState, ready, reason) {
  const nextState = ready ? null : reason;
  if (previousState === nextState) return null;
  return {
    state: nextState,
    message:
      nextState === null ? "now conformal-ready" : `not conformal-ready: ${nextState}`,
  };
}

/// Whether a read failure should be logged now. The first failure of a session
/// (`lastLoggedMs === null`, never logged) logs immediately; after that, at
/// most one line per `intervalMs`. A host that resets a stalled frame's stream
/// surfaces a WebTransportError here every frame, which after the first is
/// expected, not per-frame news.
export function shouldLogReadFailure(nowMs, lastLoggedMs, intervalMs) {
  return lastLoggedMs === null || nowMs - lastLoggedMs >= intervalMs;
}
