/** Normalizes the host's typed per-connection video delivery snapshot. */
export function normalizeVideoDelivery(message) {
  const mode = ["normal", "degraded", "suspended"].includes(message?.mode)
    ? message.mode
    : "suspended";
  const budgetBytesPerSecond = Number.isFinite(message?.budgetBytesPerSecond)
    ? Math.max(0, message.budgetBytesPerSecond)
    : 0;
  return {
    mode,
    reason: message?.reason === "bandwidth" ? "bandwidth" : "unknown",
    budgetBytesPerSecond,
  };
}

/** Viewer banner for intentional bandwidth shedding, distinct from a stall. */
export function bandwidthBannerText(state) {
  if (state.mode === "normal") return null;
  if (state.mode === "suspended") return "video suspended — bandwidth";
  const megabits = (state.budgetBytesPerSecond * 8) / 1_000_000;
  return `video degraded — bandwidth (${megabits.toFixed(1)} Mbit/s)`;
}

/** A host-declared suspension must not trigger stall recovery or stall paint. */
export function stallWatchEnabled(state) {
  return state.mode !== "suspended";
}
