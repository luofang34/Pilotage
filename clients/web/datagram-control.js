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

function releaseWriterLock(writer) {
  try {
    writer.releaseLock();
  } catch {
    // Session teardown may have invalidated the writer before the run ended.
  }
}

/** Interrupts one pending backpressure wait without closing the send stream.
 *
 *  `waitFor` resolves `"ready"`, `"stopped"` (deliberate teardown), or
 *  `"writer-failed"` (the writer's ready promise REJECTED — the datagram
 *  channel itself is dead). The two non-ready outcomes must stay distinct:
 *  a stop is silent by design, while a failed channel with control
 *  authority still held leaves the vehicle enacting the last command, so
 *  the caller must release authority loudly, never just exit. */
export function createDatagramRunStop() {
  let stopped = false;
  let wake = null;
  return {
    stop() {
      stopped = true;
      wake?.("stopped");
    },
    waitFor(writerReady) {
      if (stopped) return Promise.resolve("stopped");
      return new Promise((resolve) => {
        let settled = false;
        const finish = (outcome) => {
          if (settled) return;
          settled = true;
          wake = null;
          resolve(outcome);
        };
        wake = finish;
        Promise.resolve(writerReady).then(
          () => finish("ready"),
          () => finish("writer-failed"),
        );
      });
    },
  };
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
    releaseWriterLock(writer);
    return { ok: false, reason: "inactive-session" };
  }
  const completion = Promise.resolve()
    .then(() => (lifecycle.isActive(token) ? run(writer) : undefined))
    .catch((error) => {
      if (lifecycle.isActive(token)) onError(error);
    })
    .finally(() => {
      lifecycle.untrackWriter(token, writer);
      releaseWriterLock(writer);
    });
  return { ok: true, completion };
}
