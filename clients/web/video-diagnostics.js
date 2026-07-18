// Pure decision logic for the demo viewer's video diagnostics: log a
// per-source conformal-readiness change once (not per frame), coalesce a
// burst of uni-stream read failures, and decide whether a video-reader
// exit should resume or give up. Kept side-effect-free so it is testable
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

/// Whether a read failure should be logged now: at most one line per
/// `intervalMs`. A host that resets a stalled frame's stream surfaces a
/// WebTransportError here every frame, which is expected, not per-frame news.
export function shouldLogReadFailure(nowMs, lastLoggedMs, intervalMs) {
  return nowMs - lastLoggedMs >= intervalMs;
}

/// The verdict for a supervised video-reader exit. A reader that ran at
/// least `minUptimeMs` before exiting was a transient interruption and
/// resumes with the exit counter cleared; one that exits in under that
/// window increments the counter, and once it reaches `maxImmediate` the
/// incoming-streams side is judged gone — give up rather than spin.
export function restartVerdict(uptimeMs, immediateExits, { minUptimeMs, maxImmediate }) {
  const ranBriefly = uptimeMs < minUptimeMs;
  const nextImmediateExits = ranBriefly ? immediateExits + 1 : 0;
  return {
    ranBriefly,
    immediateExits: nextImmediateExits,
    giveUp: nextImmediateExits >= maxImmediate,
  };
}
