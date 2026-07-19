// Capability and lifecycle boundary for the browser's control-datagram writer.

/** Acquires the available datagram send stream without assuming an engine. */
export function acquireDatagramWriter(datagrams) {
  let writable;
  try {
    writable =
      typeof datagrams?.createWritable === "function"
        ? datagrams.createWritable()
        : datagrams?.writable;
    if (!writable || typeof writable.getWriter !== "function") {
      return { ok: false, reason: "send-stream-unavailable" };
    }
    return { ok: true, writer: writable.getWriter() };
  } catch (cause) {
    return { ok: false, reason: "writer-acquisition-failed", cause };
  }
}

/**
 * Acquires and registers one control writer, then runs it under the session
 * lifecycle so replacement or teardown aborts the writer before authority can
 * leak into a stale session.
 */
export function startDatagramControl({ datagrams, lifecycle, token, run, onError }) {
  const acquired = acquireDatagramWriter(datagrams);
  if (!acquired.ok) return acquired;
  const { writer } = acquired;
  if (!lifecycle.trackWriter(token, writer)) {
    return { ok: false, reason: "inactive-session" };
  }
  const completion = Promise.resolve()
    .then(() => (lifecycle.isActive(token) ? run(writer) : undefined))
    .catch((error) => {
      if (lifecycle.isActive(token)) onError(error);
    })
    .finally(() => lifecycle.untrackWriter(token, writer));
  return { ok: true, completion };
}
