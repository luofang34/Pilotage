function finiteFields(value, names) {
  return value !== null && value !== undefined && names.every((name) => Number.isFinite(value[name]));
}

const SESSION_COPY = Object.freeze({
  connecting: ["telemetry Missing (connecting)", "connecting"],
  awaiting: ["telemetry Missing (awaiting first sample)", "connected — awaiting telemetry"],
  disconnected: ["telemetry Missing (disconnected)", "disconnected"],
  failed: ["telemetry Missing (session failed)", "connection failed"],
});

export function setTelemetrySessionState(elements, phase) {
  const copy = SESSION_COPY[phase];
  if (!copy) throw new RangeError(`unknown telemetry session phase ${phase}`);
  elements.telemetry.textContent = copy[0];
  elements.overlay.textContent = copy[1];
}

export function formatTelemetrySummary(sample) {
  const arm = { 0: "arm: unknown", 1: "DISARMED", 2: "ARMED" }[
    sample.avionics?.armState ?? 0
  ] ?? "arm: invalid";
  let pose = "pose Missing";
  if (sample.pose !== null && sample.pose !== undefined) {
    pose = finiteFields(sample.pose, ["xM", "yM", "headingRad"])
      ? `pose x=${sample.pose.xM.toFixed(2)}m y=${sample.pose.yM.toFixed(2)}m heading=${sample.pose.headingRad.toFixed(2)}rad`
      : "pose Invalid";
  }
  let velocity = "velocity Missing";
  if (sample.velocity !== null && sample.velocity !== undefined) {
    velocity = finiteFields(sample.velocity, ["linearXMps", "angularRadS"])
      ? `v=${sample.velocity.linearXMps.toFixed(2)}m/s w=${sample.velocity.angularRadS.toFixed(2)}rad/s`
      : "velocity Invalid";
  }
  return `${arm} | ${pose} | ${velocity}`;
}
