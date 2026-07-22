// Video-source identity belongs in one binding table: a source must never be
// silently redirected to another camera when the viewer grows a new feed.

export const VIDEO_SOURCE_ID = Object.freeze({
  FPV: 0,
  CHASE: 1,
  GIMBAL: 2,
});

const SOURCE_CANVASES = Object.freeze([
  [VIDEO_SOURCE_ID.FPV, "video"],
  [VIDEO_SOURCE_ID.CHASE, "chaseVideo"],
  [VIDEO_SOURCE_ID.GIMBAL, "gimbalVideo"],
]);

/** Binds every assigned source id to its own live canvas and 2D context. */
export function bindVideoTargets(documentRoot) {
  const targets = Object.create(null);
  for (const [sourceId, elementId] of SOURCE_CANVASES) {
    const canvas = documentRoot.getElementById(elementId);
    const context = canvas?.getContext("2d") ?? null;
    if (!canvas || !context) {
      throw new Error(`video source ${sourceId} canvas #${elementId} is unavailable`);
    }
    canvas.dataset.videoSourceId = String(sourceId);
    targets[sourceId] = Object.freeze({ canvas, ctx: context });
  }
  return Object.freeze(targets);
}

/** Resolves an assigned source, reporting only genuinely unknown identities. */
export function resolveVideoTarget(targets, sourceId, onUnknown) {
  const target = targets[sourceId];
  if (target) return target;
  onUnknown(sourceId);
  return null;
}
