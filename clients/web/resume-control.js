// Same-session control resumption after the input-loss latch relinquishes
// authority. The gate re-arm and every live probe stay in this module so the
// browser lifecycle and its race tests execute the same ordering.

/**
 * Re-arms control, waits for the release and writer to settle, then requests
 * fresh authority without replacing the live transport.
 */
export async function resumeSessionControl({
  gate,
  releases,
  controlSettled,
  isSessionLive,
  announceActivation,
  requestLeases,
  surrender,
}) {
  gate.reset();
  await releases.settled();
  await controlSettled();
  if (!isSessionLive() || gate.isLatched()) {
    return { requested: false, interrupted: true };
  }

  announceActivation();
  try {
    await requestLeases();
  } catch (error) {
    surrender();
    throw error;
  }
  if (!isSessionLive() || gate.isLatched()) {
    surrender();
    return { requested: true, interrupted: true };
  }
  return { requested: true, interrupted: false };
}

/** Why the authority table refused to plan the resume's motion request.
 *  Denial is terminal for the session, and a still-granted slot means the
 *  release acknowledgement never landed (the host watchdog covers the lost
 *  ack host-side). Neither resolves in place, so the resume fails loudly
 *  with this reason instead of waiting for a response that cannot come. */
export function resumeRefusalReason({ granted, denied }) {
  const reason = denied
    ? "motion lease denied this session"
    : granted
      ? "release unacknowledged"
      : "no motion request plannable";
  return `${reason}; press Connect for a fresh session`;
}

/** Decides how a mid-session motion grant completes a pending resume. */
export function resumeGrantDecision({ pending, granted, sessionLive, mayPublish }) {
  if (!pending) return "unrelated";
  if (!granted) return "denied";
  if (!sessionLive || !mayPublish) return "surrender";
  return "start";
}
