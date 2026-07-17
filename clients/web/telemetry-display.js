import { ROLE, stampFaultForRole } from "./telemetry-ingress.js";

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

export function formatTelemetrySummary(sample, fcView = null) {
  // Arm state is FC-owned and arrives as its own stamped report,
  // aged by the FC-state tracker: missing before any report, stale
  // after heartbeat loss, and never refreshed by duplicate reports.
  let arm = "arm: missing";
  if (fcView !== null) {
    arm = fcView.stale
      ? "arm: stale"
      : ({ 0: "arm: unknown", 1: "DISARMED", 2: "ARMED" }[fcView.armState] ?? "arm: invalid");
  }
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
  // Simulator truth renders only under an explicit oracle label — it is
  // not the operational pose and must never look like one — and only
  // when the COMPLETE role-specific stamp validates (simulation-truth
  // role, simulation clock, known integrity, well-formed identity) and
  // position availability holds.
  let truth = "";
  const truthSample = sample.simTruth;
  if (truthSample !== null && truthSample !== undefined) {
    const posNed = truthSample.posNed;
    const positionAvailable = ((truthSample.validFlags ?? 0) & 0b100) !== 0;
    const stampValid = stampFaultForRole(truthSample.stamp, ROLE.SIMULATION_TRUTH) === null;
    truth =
      stampValid &&
      positionAvailable &&
      Array.isArray(posNed) &&
      posNed.every((value) => Number.isFinite(value))
        ? ` | SIM truth (oracle): n=${posNed[0].toFixed(2)}m e=${posNed[1].toFixed(2)}m d=${posNed[2].toFixed(2)}m`
        : " | SIM truth (oracle): Unavailable";
  }
  return `${arm} | ${pose} | ${velocity}${truth}`;
}
