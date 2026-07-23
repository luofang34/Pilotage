// Per-source video frame-arrival freshness. A slot whose frames stop
// arriving — a host writer death, a retired source, a broken route — must
// SAY so on the slot instead of freezing on the last picture. Pure state
// transitions; the readout owns the clock, the canvases, and the log.

/** A source is stalled once no frame has arrived for this long. Frames run
 *  at camera rate (tens of Hz); two seconds of silence is unambiguous. */
export const VIDEO_STALL_THRESHOLD_MS = 2000;

/** Fresh tracking state: no sources seen, none stalled. */
export function createVideoFreshness() {
  return { lastFrameMs: new Map(), stalled: new Set() };
}

/** Records one delivered frame for `sourceId`. Returns `true` when the
 *  source was stalled and this frame RECOVERED it (the caller logs the
 *  transition; the frame itself repaints the slot). */
export function noteVideoFrame(freshness, sourceId, nowMs) {
  freshness.lastFrameMs.set(sourceId, nowMs);
  return freshness.stalled.delete(sourceId);
}

/** Advances the stall watch: returns the sources that JUST crossed the
 *  threshold (one transition each — the caller banners and logs them once;
 *  already-stalled sources are not repeated). */
export function newlyStalledSources(freshness, nowMs, thresholdMs = VIDEO_STALL_THRESHOLD_MS) {
  const entering = [];
  for (const [sourceId, lastMs] of freshness.lastFrameMs) {
    if (nowMs - lastMs < thresholdMs || freshness.stalled.has(sourceId)) continue;
    freshness.stalled.add(sourceId);
    entering.push(sourceId);
  }
  return entering;
}

/** True only when every source that has painted a frame is now stalled. */
export function allVideoSourcesStalled(freshness) {
  return freshness.lastFrameMs.size > 0 && freshness.stalled.size === freshness.lastFrameMs.size;
}
