/** Retry policy for media attachment while every observed source is stalled. */
export class MediaRecoveryGate {
  constructor(retryMs) {
    this.retryMs = retryMs;
    this.lastRequestMs = null;
  }

  shouldRequest(allSourcesStalled, nowMs) {
    if (!allSourcesStalled) return false;
    if (this.lastRequestMs !== null && nowMs - this.lastRequestMs < this.retryMs) return false;
    this.lastRequestMs = nowMs;
    return true;
  }

  notePaintedFrame() {
    this.lastRequestMs = null;
  }

  reset() {
    this.lastRequestMs = null;
  }
}
