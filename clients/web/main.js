// Pilotage demo browser viewer wiring.

import { TransportSessionLifecycle } from "./transport-session.js";
import { createControlGate } from "./control-gate.js";
import { createReleaseTracker } from "./lease-release.js";
import { createActionTracker } from "./action-tracker.js";
import { whenVisible } from "./session-discovery.js";
import { createSessionTransport } from "./session-transport.js";
import { createSessionBootstrap } from "./session-bootstrap.js";
import { createControlLoop } from "./control-loop.js";
import { createCockpitReadout } from "./cockpit-readout.js";
import { loadCompositionForPage } from "./instrument-startup.js";

const VEHICLE_ID = 1n;
const INSTRUMENT_SOURCE_ID = 1n;
const SIM_COHERENCE_LIMIT_NS = 300_000_000n;
const MOTION_SCOPE = "vehicle.motion";
const DIRECT_SCOPE = "vehicle.motion.direct";
const CONTROL_HZ = 30;
const GIMBAL_SCOPE = "vehicle.gimbal";
const SIM_LIFECYCLE_SCOPE = "sim.lifecycle";
const FRAME_REJECTION_UPLINK_IDLE = 18;

const els = {
  host: document.getElementById("host"),
  port: document.getElementById("port"),
  certHash: document.getElementById("certHash"),
  connectBtn: document.getElementById("connectBtn"),
  resumeBtn: document.getElementById("resumeBtn"),
  status: document.getElementById("status"),
  overlay: document.getElementById("overlay"),
  telemetry: document.getElementById("telemetry"),
  gamepad: document.getElementById("gamepad"),
  flightMode: document.getElementById("flightMode"),
};

// The instrument stages come from the screen composition (ADR-0032).
// layout.js evaluates first and builds their figures from the same
// memoized promise, so the canvases exist before the readout resolves
// them. A failed enumeration keeps the rest of the client available and
// leaves a page-level display failure visible.
const { slots: instrumentSlots, fault: instrumentStartupFault } =
  await loadCompositionForPage();

const state = {
  transport: null,
  sessionWriter: null,
  sessionId: 0,
  principalId: 0n,
  sequence: 0,
  startNanos: BigInt(Date.now()) * 1_000_000n,
  selectedPadId: null,
  pendingReset: false,
  pendingFpvToggle: false,
  lastFcVerdictLogged: null,
  lastFcView: null,
  pressWatchToken: null,
  fpvActive: false,
  actionTracker: createActionTracker(),
  announcedActivationRevision: null,
  motionScope: MOTION_SCOPE,
  pendingMotionScope: null,
  fpvHeading: 0,
  lastDirectFrameMs: 0,
  lifecycle: { pendingPress: false },
  advertisedScopes: [],
  connected: false,
  controlCompletion: null,
  stopControlRun: null,
  resumePendingToken: null,
  resumeGimbalLease: false,
  lastFrameRejectionLogged: null,
  gimbalSequence: 0,
  controlShell: null,
  skippedVideoFrames: 0,
  supersededVideoFrames: 0,
  h264UnavailableLogged: false,
  droppedIdentityFrames: 0,
  lastAssociation: null,
};

const transportSessions = new TransportSessionLifecycle();
const controlGate = createControlGate({ isFocused: () => document.hasFocus() });
const releaseTracker = createReleaseTracker();

let control;
let bootstrap;
let transport;
let controlStarted = Promise.resolve();

const readout = createCockpitReadout({
  state,
  els,
  instrumentSlots,
  instrumentStartupFault,
  transportSessions,
  vehicleId: VEHICLE_ID,
  instrumentSourceId: INSTRUMENT_SOURCE_ID,
  coherenceLimitNanos: SIM_COHERENCE_LIMIT_NS,
  authorityFor: (...args) => control.authorityFor(...args),
  controlGate,
  dispatchAuthorityEnvelope: (...args) => control.dispatchAuthorityEnvelope(...args),
  handleFrameRejected: (...args) => control.handleFrameRejected(...args),
  handleTransportClosed: (...args) => transport.handleTransportClosed(...args),
  requestMediaAttach: (...args) => bootstrap.requestMediaAttach(...args),
  requestReconnect: () => transport.reconnect.requestConnect(),
});

control = createControlLoop({
  state,
  els,
  transportSessions,
  controlGate,
  releaseTracker,
  vehicleId: VEHICLE_ID,
  motionScope: MOTION_SCOPE,
  directScope: DIRECT_SCOPE,
  gimbalScope: GIMBAL_SCOPE,
  lifecycleScope: SIM_LIFECYCLE_SCOPE,
  frameRejectionUplinkIdle: FRAME_REJECTION_UPLINK_IDLE,
  controlHz: CONTROL_HZ,
  log: readout.log, surface: readout.surface,
  updateControlReadout: readout.updateControlReadout,
  reportSuppressedPresses: readout.reportSuppressedPresses,
  currentTelemetryHeading: readout.currentTelemetryHeading,
  lengthDelimit: (...args) => bootstrap.lengthDelimit(...args),
  maybeAnnounceProfileActivation: (...args) => bootstrap.maybeAnnounceProfileActivation(...args),
  requestReconnect: () => transport.reconnect.requestConnect(),
});

bootstrap = createSessionBootstrap({
  state,
  surface: readout.surface,
  transportSessions,
  motionScope: MOTION_SCOPE,
  controlStarted: () => controlStarted,
  log: readout.log,
  authorityFor: control.authorityFor,
  executeLeaseAction: control.executeLeaseAction,
  velocityCapabilityFor: control.velocityCapabilityFor,
  handleActionResult: control.handleActionResult,
  applyLeaseResponse: control.applyLeaseResponse,
});

transport = createSessionTransport({
  state,
  els,
  transportSessions,
  controlGate,
  releaseTracker,
  motionScope: MOTION_SCOPE,
  log: readout.log, surface: readout.surface,
  readout,
  bootstrap,
  control,
});

window.addEventListener("pagehide", readout.dispose, { once: true });
window.addEventListener("keydown", (event) => control.forwardKey(event, true));
window.addEventListener("keyup", (event) => control.forwardKey(event, false));
window.addEventListener("gamepaddisconnected", control.gamepadDisconnected);
// The arm button drives the SAME path as the keyboard: it synthesizes
// the bound key, so every gate, lease, and profile rule between a key
// press and the vehicle applies unchanged. Arming asks once to confirm
// — a click is easier to land by accident than an Enter press.
{
  const armBtn = document.getElementById("armBtn");
  const disarmBtn = document.getElementById("disarmBtn");
  let armPending = null;
  const pressKey = (key) => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
    window.setTimeout(() => {
      window.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true }));
    }, 60);
  };
  armBtn.addEventListener("click", () => {
    if (armPending === null) {
      armBtn.textContent = "confirm ARM?";
      armPending = window.setTimeout(() => {
        armPending = null;
        armBtn.textContent = "ARM";
      }, 3000);
      return;
    }
    window.clearTimeout(armPending);
    armPending = null;
    armBtn.textContent = "ARM";
    pressKey("Enter");
  });
  disarmBtn.addEventListener("click", () => pressKey("Backspace"));
}

document.getElementById("fpvBtn").addEventListener("click", () => {
  state.pendingFpvToggle = true;
});
document.getElementById("resetBtn").addEventListener("click", () => {
  state.pendingReset = true;
});
els.connectBtn.addEventListener("click", () => transport.reconnect.requestConnect());
els.resumeBtn.addEventListener("click", () => void control.resumeControlInPlace());

transport.applyUrlParams();
const instrumentsStarted = readout.startInstruments();
controlStarted = control.startControl();

const params = new URLSearchParams(window.location.search);
if (
  params.get("autoconnect") === "1" &&
  els.host.value &&
  els.port.value &&
  els.certHash.value
) {
  whenVisible(document, () => {
    instrumentsStarted.finally(() => transport.reconnect.requestConnect());
  });
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") {
    transport.reconnect.notifyVisible();
    control.showResumeAffordance();
  }
});

window.addEventListener("focus", control.showResumeAffordance);
window.addEventListener("blur", () => {
  controlGate.latchInputLoss();
  const token = transportSessions.currentToken();
  if (token) control.suspendControlForInputLoss(token);
  state.controlShell?.clearKeys();
  control.showResumeAffordance();
});
