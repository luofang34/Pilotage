// Pure decision logic for the demo viewer's connection auto-recovery: when a
// WebTransport session drops unexpectedly (a backgrounded tab that the browser
// froze, an idle timeout, a network blip), decide whether and when to
// reconnect. Kept side-effect-free so it is testable apart from the transport
// plumbing in main.js.

/// The backoff before the next reconnect attempt: exponential in `attempts`,
/// capped at `maxMs`. `attempts` is the count already made (0 for the first).
export function reconnectDelayMs(attempts, { baseMs, maxMs }) {
  const scaled = baseMs * 2 ** attempts;
  return Math.min(maxMs, scaled);
}

/// Whether to attempt a reconnect now, and whether to give up. Reconnect only
/// when the user wants to stay connected, no session is currently active, the
/// page is visible (a hidden/frozen tab would just drop again — wait until the
/// user returns), and the attempt budget is not spent.
export function reconnectDecision({ wanted, active, visible, attempts, maxAttempts }) {
  if (attempts >= maxAttempts) return { attempt: false, giveUp: true };
  const attempt = wanted && !active && visible;
  return { attempt, giveUp: false };
}
