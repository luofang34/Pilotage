// The authority side of one connect, in the ORDER the safety argument
// depends on (CTRL-04, #147) — owned here so main.js and the lifecycle
// tests execute the same production orchestration, and a regression in
// the ordering fails the test rather than only re-deriving it:
//
//   1. On a manual connect, re-arm the input-loss gate SYNCHRONOUSLY,
//      before the first await: a blur landing during any later await of
//      this connect latches and stays latched.
//   2. Await any in-flight release before the transport work supersedes
//      the old session — the old session's stream reader is what
//      delivers the acknowledgement (bounded; a lost ack degrades to
//      the host watchdog).
//   3. Open the transport and run bootstrap with the lease decision as
//      a LIVE probe, evaluated at the ServerWelcome moment immediately
//      before the one LeaseRequest emission.
//   4. After a grant, re-check the gate: a blur that raced the grant
//      releases the lease immediately on the new reliable stream;
//      otherwise control starts. No grant → telemetry-only.

/**
 * @param {object} deps
 * @param {boolean} deps.manual explicit user connect (only path to control)
 * @param {{ reset: () => void, isLatched: () => boolean }} deps.gate
 * @param {{ settled: () => Promise<any> }} deps.releases
 * @param {(leaseProbe: () => boolean) => Promise<object>} deps.openAndBootstrap
 *   transport construction + handshake + bootstrap; must pass `leaseProbe`
 *   through as the reader's live `requestLease` and resolve to a session
 *   object with `completed` (bootstrap finished) and `leaseGranted`;
 *   anything not `completed` is returned to the caller untouched.
 * @param {(session: object) => boolean|void} deps.startControl
 * @param {(session: object) => void} deps.controlUnavailable
 * @param {(session: object) => void} deps.releaseLease
 * @param {(session: object, manual: boolean) => void} deps.telemetryOnly
 */
export async function negotiateSessionAuthority({
  manual,
  gate,
  releases,
  openAndBootstrap,
  startControl,
  controlUnavailable,
  releaseLease,
  telemetryOnly,
}) {
  if (manual) {
    gate.reset(); // synchronous, BEFORE the first await
    await releases.settled();
  }
  const session = await openAndBootstrap(() => manual && !gate.isLatched());
  if (!session || !session.completed) return session;
  if (session.leaseGranted) {
    if (!gate.isLatched()) {
      if (startControl(session) === false) controlUnavailable(session);
    } else {
      // The blur raced the grant: input loss latched after the lease
      // request was already emitted. Surrender immediately.
      releaseLease(session);
    }
  } else {
    telemetryOnly(session, manual);
  }
  return session;
}
