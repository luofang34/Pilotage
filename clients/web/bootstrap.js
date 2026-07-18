// The bootstrap reader: consumes the length-delimited handshake stream until
// the session is established, with the lease decision made HERE — the one
// place a LeaseRequest can be emitted — so "an auto-reconnect never
// re-requests motion authority" is a property of the real reader loop, not
// of a flag some caller promises to pass.
//
// Side-effect free by construction: the stream, the envelope decoder, the
// liveness check, and the lease send are all injected, so the loop is
// testable against fakes while main.js runs it against the real transport.

/** Reads bootstrap frames until the session is established.
 *
 * With `requestLease` it sends a `LeaseRequest` on `ServerWelcome` and
 * completes on `LeaseResponse` (the control path); without it, bootstrap
 * completes at `ServerWelcome` and no lease is requested, so the session is
 * telemetry/video only.
 *
 * Returns `{ completed, welcomed }`. `welcomed` distinguishes where an
 * incomplete bootstrap ended (before or after `ServerWelcome`) — useful
 * telemetry, but NOT a rejection signal: an untyped stream end is
 * retryable wherever it lands, because the host's close carries no wire
 * payload and a pre-welcome EOF is indistinguishable from an early
 * network blip. Only a future typed protocol rejection may classify as
 * non-retryable (#146).
 *
 * @param {object} deps
 * @param {{ read: () => Promise<{value?: Uint8Array, done: boolean}> }} deps.reader
 *   the bootstrap stream reader
 * @param {(bytes: Uint8Array) => ({kind: string, consumed: number} | null)} deps.decode
 *   length-delimited envelope decoder; null means "need more bytes"
 * @param {() => boolean} deps.isActive superseded sessions stop reading
 * @param {(decoded: object) => void} deps.onMessage decoded-frame sink
 * @param {boolean | (() => boolean)} deps.requestLease whether to request
 *   motion authority. Accepts a FUNCTION evaluated at the `ServerWelcome`
 *   moment — the instant before the one `LeaseRequest` emission — so a
 *   live condition (an input-loss latch set during an earlier await of
 *   this very connect) is honored, not a stale flag captured when the
 *   connect began.
 * @param {() => Promise<boolean>} deps.sendLeaseRequest writes the
 *   `LeaseRequest`; false means the session was superseded mid-send
 */
export async function runBootstrapReader({
  reader,
  decode,
  isActive,
  onMessage,
  requestLease,
  sendLeaseRequest,
}) {
  let pending = new Uint8Array(0);
  let welcomed = false;
  let sentLease = false;
  for (;;) {
    const { value, done } = await reader.read();
    if (!isActive()) return { completed: false, welcomed };
    if (done) return { completed: false, welcomed };
    pending = appendBytes(pending, value);
    for (;;) {
      const decoded = decode(pending);
      if (!decoded) break;
      pending = pending.subarray(decoded.consumed);
      onMessage(decoded);
      if (decoded.kind === "ServerWelcome") {
        welcomed = true;
        const wantsLease =
          typeof requestLease === "function" ? requestLease() : requestLease;
        if (!wantsLease) {
          // Telemetry/video only; control stays suspended.
          return { completed: true, welcomed };
        }
        if (!sentLease) {
          sentLease = true;
          if (!(await sendLeaseRequest())) return { completed: false, welcomed };
        }
      }
      if (decoded.kind === "LeaseResponse") {
        return { completed: true, welcomed };
      }
    }
  }
}

function appendBytes(existing, incoming) {
  const out = new Uint8Array(existing.length + incoming.length);
  out.set(existing, 0);
  out.set(incoming, existing.length);
  return out;
}
