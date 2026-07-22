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

/** The FC-owned arm state as one readout token — shared by the telemetry
 *  line and the control readout so the two can never disagree. Arm state
 *  arrives as its own stamped report, aged by the FC-state tracker:
 *  missing before any report, stale after heartbeat loss, and never
 *  refreshed by duplicate reports. */
export function fcArmToken(fcView) {
  if (fcView === null || fcView === undefined) return "arm: missing";
  let arm = fcView.stale
    ? "arm: stale"
    : ({ 0: "arm: unknown", 1: "DISARMED", 2: "ARMED" }[fcView.armState] ?? "arm: invalid");
  // Enactment truth: the FC refused the most recent commanded
  // arm/disarm. Without this marker, "command accepted" and a stubborn
  // DISARMED readout are indistinguishable from a dead control.
  const verdict = fcView.lastCommand;
  if (!fcView.stale && verdict && verdict.result !== 0) {
    arm += ` (FC refused ${verdict.arm ? "arm" : "disarm"}: result ${verdict.result})`;
  }
  return arm;
}

export function formatTelemetrySummary(sample, fcView = null) {
  const arm = fcArmToken(fcView);
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
