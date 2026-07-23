// Escalating recovery for a video pipeline whose frames stopped arriving.
//
// Re-attach requests ride the live session and fix routing-level breaks
// (writer retirement, lost registration). But when the SESSION's host→client
// stream direction itself is dead — the wedge family where datagrams still
// flow while every stream write blocks — re-attaches go unanswered forever
// and only a fresh connection recovers. The gate escalates: a bounded number
// of paced re-attaches, then one session-restart decision, so a freeze the
// operator used to fix by clicking Connect fixes itself.

/** Attach requests that may go unanswered (no painted frame after each)
 *  before the session itself is judged dead and restarted. */
export const MEDIA_RESTART_AFTER_ATTEMPTS = 3;

/** Retry policy for media recovery while every observed source is stalled. */
export class MediaRecoveryGate {
  constructor(retryMs) {
    this.retryMs = retryMs;
    this.lastRequestMs = null;
    this.attempts = 0;
    this.restarted = false;
  }

  /** One stall-watch tick's decision: `"attach"` to request re-attachment,
   *  `"restart"` (at most once until frames recover) to replace the session,
   *  or `null` to wait. Requests are paced `retryMs` apart. */
  decide(allSourcesStalled, nowMs) {
    if (!allSourcesStalled || this.restarted) return null;
    if (this.lastRequestMs !== null && nowMs - this.lastRequestMs < this.retryMs) return null;
    if (this.attempts >= MEDIA_RESTART_AFTER_ATTEMPTS) {
      this.restarted = true;
      return "restart";
    }
    this.lastRequestMs = nowMs;
    this.attempts += 1;
    return "attach";
  }

  /** A painted frame proves delivery works again: rearm from scratch. */
  notePaintedFrame() {
    this.lastRequestMs = null;
    this.attempts = 0;
    this.restarted = false;
  }

  /** A fresh session starts with a fresh escalation ladder. */
  reset() {
    this.notePaintedFrame();
  }
}
