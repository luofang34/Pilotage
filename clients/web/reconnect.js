// Connection auto-recovery for the demo viewer.
//
// When a WebTransport session drops unexpectedly (a backgrounded tab the
// browser froze, an idle timeout, a network blip), reconnect the TRANSPORT so
// telemetry and video come back — but never automatically re-request motion
// authority. Control stays suspended until the user explicitly re-enables it;
// that gate lives in the control loop, not here.
//
// The controller is side-effect-free except through its injected `connect`,
// `schedule`, and `cancel`, so its whole lifecycle runs under fake time and a
// fake WebTransport in tests.

import { classifyConnectFailure } from "./session-admission.js";
export { classifyConnectFailure };

/// The backoff before the next reconnect attempt: exponential in `attempts`,
/// capped at `maxMs`, with symmetric bounded jitter so a fleet of viewers that
/// dropped together does not resynchronize into a thundering herd. `attempts`
/// is the count already made (0 for the first); `jitter` in [0,1) is the
/// caller's random fraction (0 yields the un-jittered value). Once the
/// exponential reaches the cap the delay stays at the cap — a slow, capped
/// retry that continues indefinitely rather than a hard stop or a busy loop.
export function reconnectDelayMs(attempts, { baseMs, maxMs, jitterRatio = 0.25 }, jitter = 0) {
  const exponential = Math.min(maxMs, baseMs * 2 ** attempts);
  const spread = exponential * jitterRatio;
  const offset = (jitter * 2 - 1) * spread; // [0,1) -> [-spread, +spread)
  return Math.max(0, Math.round(exponential + offset));
}

/// Creates a reconnect controller.
///
/// Dependencies:
///   - `connect({ manual })` — performs one connect attempt and resolves to
///     `{ ok }` or `{ ok: false, failure }`. `manual` is true only for a
///     user-initiated Connect; an auto-reconnect passes false so `connect`
///     restores transport/telemetry/video WITHOUT requesting motion authority.
///   - `schedule(delayMs, cb) -> handle` / `cancel(handle)` — the timer.
///   - `isVisible()` / `isActive()` — page visibility and whether a session is
///     already up.
///   - `random()` — jitter source in [0,1).
///   - `log(message)` — user-facing status line.
export function createReconnectController({
  connect,
  schedule,
  cancel,
  isVisible,
  isActive,
  random = () => 0,
  log = () => {},
  baseMs = 1000,
  maxMs = 15000,
  jitterRatio = 0.25,
}) {
  const state = { wanted: false, attempts: 0, timer: null, stopped: false, halted: false };
  let currentAttempt = null;

  function clearTimer() {
    if (state.timer !== null) {
      cancel(state.timer);
      state.timer = null;
    }
  }

  function scheduleNext(rawFailure) {
    if (state.stopped || state.halted) return;
    if (!state.wanted) return; // the user never asked to be connected
    const failure = classifyConnectFailure(rawFailure);
    if (rawFailure && !failure.retryable) {
      // LATCH the halt: a non-retryable failure ends auto-recovery until
      // an explicit Connect. Without the latch, any later trigger with no
      // failure attached — a visibilitychange, a stray drop notification —
      // would classify as retryable and silently restart the loop.
      state.halted = true;
      clearTimer();
      log(`connection cannot be auto-recovered (${failure.kind}); press Connect to retry`);
      return;
    }
    if (state.timer !== null) return;
    if (isActive()) return; // already connected
    if (!isVisible()) return; // hidden/frozen tab would just drop again — wait for visibility
    const delay = reconnectDelayMs(state.attempts, { baseMs, maxMs, jitterRatio }, random());
    state.timer = schedule(delay, () => {
      state.timer = null;
      state.attempts += 1;
      log(`connection lost — auto-reconnecting (attempt ${state.attempts})...`);
      void attempt(false);
    });
  }

  async function attempt(manual) {
    if (state.stopped || !state.wanted) return;
    const identity = {};
    currentAttempt = identity;
    const outcome = await connect({ manual });
    if (state.stopped || currentAttempt !== identity) return;
    if (outcome && outcome.ok) return; // success: notifyBootstrapComplete resets the backoff
    scheduleNext(outcome && outcome.failure);
  }

  return {
    /// The user clicked Connect: an explicit, fresh start that resets the
    /// backoff and requests control.
    requestConnect() {
      state.wanted = true;
      state.stopped = false;
      state.halted = false;
      state.attempts = 0;
      clearTimer();
      void attempt(true);
    },
    /// The transport dropped while we still want a session; schedule recovery.
    notifyDropped(failure) {
      scheduleNext(failure);
    },
    /// Application bootstrap finished (ServerWelcome + LeaseResponse). Only now
    /// is the connection proven good, so the backoff resets HERE — never at
    /// transport.ready, which precedes bootstrap and would retry a
    /// bootstrap/protocol failure forever at the base interval.
    notifyBootstrapComplete() {
      state.attempts = 0;
    },
    /// The tab became visible again — retry a reconnect that was deferred while
    /// it was hidden.
    notifyVisible() {
      scheduleNext();
    },
    /// The user asked to disconnect, or teardown: stop trying.
    stop() {
      state.stopped = true;
      state.wanted = false;
      clearTimer();
    },
    /// Inspection for tests.
    snapshot() {
      return {
        wanted: state.wanted,
        attempts: state.attempts,
        pending: state.timer !== null,
        halted: state.halted,
      };
    },
  };
}
