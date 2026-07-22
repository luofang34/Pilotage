// Tracks one in-flight explicit lease release and its acknowledgement: the
// client treats authority as relinquished when the host's `LeaseReleased`
// arrives, and the next authority attempt waits — bounded — for settlement so
// a fresh `LeaseRequest` cannot race the release into an `AlreadyHeld` denial.
// The host's silence watchdog
// remains the independent backup: the bounded wait means a lost
// acknowledgement degrades to the watchdog path instead of hanging the
// reconnect.

/**
 * @param {object} [deps]
 * @param {number} [deps.timeoutMs] bound on waiting for the acknowledgement
 * @param {(cb: () => void, ms: number) => any} [deps.schedule]
 * @param {(handle: any) => void} [deps.cancel]
 */
export function createReleaseTracker({
  timeoutMs = 1200,
  schedule = (cb, ms) => setTimeout(cb, ms),
  cancel = (handle) => clearTimeout(handle),
} = {}) {
  let pending = null;

  function settle(outcome) {
    if (!pending) return;
    const { resolve, timer } = pending;
    pending = null;
    if (timer !== null) cancel(timer);
    resolve(outcome);
  }

  return {
    /**
     * Records a release as sent; the returned promise settles with
     * "acknowledged", "timeout", or "abandoned". Idempotent while one is
     * already pending.
     */
    begin() {
      if (pending) return pending.promise;
      let resolve;
      const promise = new Promise((r) => {
        resolve = r;
      });
      const timer = schedule(() => settle("timeout"), timeoutMs);
      pending = { promise, resolve, timer };
      return promise;
    },
    /** The host acknowledged: authority is relinquished now. */
    acknowledge() {
      settle("acknowledged");
    },
    /** The transport died first; the watchdog will do the release. */
    abandon() {
      settle("abandoned");
    },
    /** Whether an acknowledgement is still outstanding. */
    isPending() {
      return pending !== null;
    },
    /**
     * Resolves once any in-flight release settles ("idle" immediately when
     * none is): the next explicit authority attempt awaits this before a lease.
     */
    settled() {
      return pending ? pending.promise : Promise.resolve("idle");
    },
  };
}
