// Pilotage demo browser viewer (ADR-0004 local-demo shortcut; ADR-0005
// WebTransport as the primary real-time transport). Not the v1 client — a
// minimal, self-contained page proving command uplink, telemetry downlink,
// and MJPEG video downlink work end to end against a real session host.
//
// Serve statically, no build step:
//   python3 -m http.server 8000
// then open http://localhost:8000/clients/web/index.html
//
// Bootstrap sequence (ADR-0005): open one WebTransport session pinned to the
// host's dev cert hash, open a bidi stream, send ClientHello, read
// ServerWelcome, send a LeaseRequest for vehicle.motion, read LeaseResponse.
// Then: accept host-initiated uni streams and dispatch on their leading
// kind-tag byte (0x01 authority-events, 0x03 one video frame with a capture
// identity header, 0x02 the legacy video frame); read telemetry-fast datagrams
// for live pose; send control-fast datagrams (bare Envelope, ControlFrame arm)
// from arrow/WASD key state.

import {
  encodeClientHelloEnvelope,
  encodeLeaseRequestEnvelope,
  encodeLeaseReleaseEnvelope,
  encodeControlFrameEnvelope,
  decodeLengthDelimitedEnvelope,
  STREAM_KIND_AUTHORITY,
  STREAM_KIND_VIDEO,
  STREAM_KIND_VIDEO_V2,
  BUTTON_EDGE_PRESSED,
} from "./wire.js";
// Real-time wire decode compiles from the host's own Rust definitions
// (ADR-0014, ADR-0020): the v2 video body (parsed and validated against the
// capture-identity contract) and the telemetry datagram both decode through
// wasm, so their byte/field layouts can never drift from the producer. JS keeps
// only transport plumbing and canvas paint. The one-time bootstrap handshake
// stays on the JS length-delimited reader.
import { decodeVideoFrameV2, decodeDatagramEnvelope } from "./instrument-runtime.js";
import { VideoIdentityTracker } from "./video-identity.js";
import { H264CanvasDecoder, H264DecoderRegistry, FOURCC_H264 } from "./video-h264.js";
import { loadInstruments, PANEL } from "./instruments.js";
import {
  coverInstrumentFailures,
  createDomFaultPresenter,
  failInstrumentSet,
  PanelHealth,
  REASON,
  renderInstrumentSet,
  startDisplayLoop,
  tickInstrumentSet,
} from "./instrument-health.js";
import { TurnDerivation } from "./turn-derivation.js";
import { formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";
import { AvionicsIngress, FcStateTracker, INCARNATION_POLICY } from "./telemetry-ingress.js";
import { TransportSessionLifecycle } from "./transport-session.js";
import { runBootstrapReader } from "./bootstrap.js";
import { createControlGate } from "./control-gate.js";
import { createReleaseTracker } from "./lease-release.js";
import { negotiateSessionAuthority } from "./connect-authority.js";
import { SnapshotAssociator, associateIfAccepted } from "./snapshot-association.js";
import { CalibrationRegistry, loadCalibrationRegistry } from "./calibration.js";
import { readinessTransition, shouldLogReadFailure } from "./video-diagnostics.js";
import { runIncomingStreamAcceptLoop } from "./uni-stream-accept.js";
import { createReconnectController } from "./reconnect.js";
import { pressedArmInputs, risingArmEdges } from "./control-edges.js";
import { startDatagramControl } from "./datagram-control.js";
import {
  PAD_GIMBAL_RESET,
  gimbalAxesFromGamepad,
  gimbalFramePlan,
  gimbalMaskedView,
  gimbalModifierHeld,
  stickShaper,
} from "./gimbal-input.js";

const VEHICLE_ID = 1n; // demo fixture: the single Gazebo vehicle this host serves.
const INSTRUMENT_SOURCE_ID = 1n; // explicit simulator adapter source; never first-packet selection.
// Aviate publishes attitude at 50 Hz and kinematics at 30 Hz. This simulator
// profile admits several stream periods plus transport jitter; an aircraft
// profile must derive its own limit from the intended function. The same
// budget bounds status-to-numeric authorization pairing in the ingress.
const SIM_COHERENCE_LIMIT_NS = 300_000_000n;
const MOTION_SCOPE = "vehicle.motion";
const CONTROL_HZ = 30; // continuous control send rate; superseded samples are droppable (ADR-0011).
const AXIS_ROLL = 0; // pilotage-input logical axis table: roll = 0 (lateral velocity, + right).
const AXIS_PITCH = 1; // pitch = 1 (forward velocity, + forward).
const AXIS_THROTTLE = 2; // throttle = 2 (rover: forward speed; quad: climb rate).
const AXIS_YAW = 3; // yaw = 3 (yaw rate, + clockwise).
const BUTTON_ARM = 0; // logical button 0: arm (adapter contract).
const BUTTON_DISARM = 1; // logical button 1: disarm.
const BUTTON_RESET = 2; // logical button 2: reset the simulation (adapter runs the reset script).
const BUTTON_FPV_TOGGLE = 3; // logical button 3: camera <-> FPV mode (adapter latches).
const GIMBAL_SCOPE = "vehicle.gimbal"; // gimbal pointing scope (GIM-01): pitch/yaw LOS rate demands, leased separately.
const GIMBAL_BUTTON_NEUTRAL = 0; // gimbal-scope logical button 0: recenter the gimbal.

const els = {
  host: document.getElementById("host"),
  port: document.getElementById("port"),
  certHash: document.getElementById("certHash"),
  connectBtn: document.getElementById("connectBtn"),
  status: document.getElementById("status"),
  overlay: document.getElementById("overlay"),
  telemetry: document.getElementById("telemetry"),
  gamepad: document.getElementById("gamepad"),
  canvas: document.getElementById("video"),
  chaseCanvas: document.getElementById("chaseVideo"),
  pfd: document.getElementById("pfd"),
  hsi: document.getElementById("hsi"),
  flightMode: document.getElementById("flightMode"),
};
const ctx = els.canvas.getContext("2d");
const chaseCtx = els.chaseCanvas.getContext("2d");
const pfdCtx = els.pfd.getContext("2d");
const hsiCtx = els.hsi.getContext("2d");
const pfdFaultPresenter = createDomFaultPresenter(els.pfd);
const hsiFaultPresenter = createDomFaultPresenter(els.hsi);

/** Session-scoped mutable state the connect flow and background loops share. */
const state = {
  transport: null,
  sessionId: 0,
  generation: 0n,
  sequence: 0,
  startNanos: BigInt(Date.now()) * 1_000_000n, // arbitrary local monotonic-ish origin for sampled_at (ADR-0009: endpoint-local, never compared raw across endpoints).
  keys: new Set(),
  prevArmInputs: new Set(),
  pendingReset: false,
  pendingFpvToggle: false,
  connected: false,
  leaseGranted: false,
  // The gimbal scope's own lease/fencing state: independent generation and
  // sequence, granted (or not) without affecting flight control.
  gimbalGeneration: 0n,
  gimbalSequence: 0,
  gimbalLeaseGranted: false,
  prevGimbalReset: false, // R3 rising-edge baseline
  gimbalStreaming: false, // a release sends one trailing neutral gimbal frame
  skippedVideoFrames: 0,
  supersededVideoFrames: 0,
  // Page-lifetime latch: wasm absence never heals without a reload (the
  // instrument module loads once at boot), so the H.264-unavailable notice
  // is worth one line, not one line per frame at stream rate.
  h264UnavailableLogged: false,
  droppedIdentityFrames: 0,
  // Latest capture-to-snapshot association verdict for an accepted frame
  // (ADR-0020). This diagnostic does not authorize a conformal overlay.
  lastAssociation: null,
};
const transportSessions = new TransportSessionLifecycle();
// Capture-identity guard for the video downlink: drops duplicate/reordered/
// stale frames so a replayed frame never displaces a newer one or refreshes
// its age (ADR-0020).
const videoIdentity = new VideoIdentityTracker();
// Bounded history of accepted aircraft snapshots, fed from the telemetry path,
// against which an accepted video frame's capture time is associated (ADR-0020).
const snapshotHistory = new SnapshotAssociator();

// The published, hash-verified camera calibrations that feed the conformal
// gate's recognized set (ADR-0021). Starts empty and fail-closed; the async
// load replaces it once the artifact is fetched and verified, and a fetch or
// verification failure simply leaves it empty (no recognized calibration).
const CALIBRATION_ARTIFACT_URL = "./sim-fpv-calibration.json";
let calibrationRegistry = new CalibrationRegistry();
loadCalibrationRegistry(CALIBRATION_ARTIFACT_URL)
  .then((registry) => {
    calibrationRegistry = registry;
  })
  .catch(() => {
    // Fail closed: keep the empty registry, conformal output stays off.
  });

// Ingestion accepts only source advancement for the selected vehicle. Drawing
// remains on requestAnimationFrame, decoupled from telemetry publication.
// Each panel latches display failures independently; a fault in the shared
// wasm backend (load/ABI/init) fails both.
function newSimulatorAvionicsIngress() {
  return new AvionicsIngress({
    vehicleId: VEHICLE_ID,
    sourceId: INSTRUMENT_SOURCE_ID,
    // SIM-only policy permits a bounded number of unseen attachment tokens.
    // Aircraft profiles pin a source-issued incarnation at authenticated
    // bootstrap and do not infer transitions from telemetry.
    incarnationPolicy: INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
    maximumSkewNanos: SIM_COHERENCE_LIMIT_NS,
  });
}

function retireSessionPresentation(phase) {
  instruments.ingress = newSimulatorAvionicsIngress();
  // FC-state freshness is session-scoped: a new session must not
  // inherit the old session's arm report or its age.
  instruments.fcState = new FcStateTracker();
  // Drop the previous session's snapshot history so a new session can never
  // associate a video frame against a stale snapshot from the old one.
  snapshotHistory.reset();
  turnDerivation.reset();
  // Close every H.264 decoder so none outlives the session that owned it; a
  // reconnect rebuilds them bound to the new session token.
  h264Registry?.closeAll();
  setTelemetrySessionState(els, phase);
}

// Browser watchdog cadence (simulator-only): a scheduling domain separate
// from requestAnimationFrame, so a stalled render loop still trips the
// liveness deadline and covers the stale frame. Passed to each panel's
// health so a tick arriving late against this cadence is recognized as
// page-wide scheduling starvation rather than a render-pipeline fault.
const WATCHDOG_INTERVAL_MS = 250;

const instruments = {
  mod: null,
  moduleFault: null,
  ingress: newSimulatorAvionicsIngress(),
  fcState: new FcStateTracker(),
  health: {
    [PANEL.PFD]: new PanelHealth({ tickIntervalMs: WATCHDOG_INTERVAL_MS }),
    [PANEL.HSI]: new PanelHealth({ tickIntervalMs: WATCHDOG_INTERVAL_MS }),
  },
};

/** The two instrument paint targets and their independent fault surfaces. */
function instrumentTargets() {
  return [
    [PANEL.PFD, pfdCtx, els.pfd, pfdFaultPresenter],
    [PANEL.HSI, hsiCtx, els.hsi, hsiFaultPresenter],
  ];
}

function log(line) {
  const time = new Date().toISOString().split("T")[1].replace("Z", "");
  els.status.textContent = `[${time}] ${line}\n${els.status.textContent}`.slice(0, 8000);
}

function nowNanos() {
  return state.startNanos + BigInt(Math.round(performance.now() * 1_000_000));
}

/** Parses the URL's `?host=&port=&cert=` params into the input boxes, if present. */
function applyUrlParams() {
  const params = new URLSearchParams(window.location.search);
  if (params.has("host")) els.host.value = params.get("host");
  if (params.has("port")) els.port.value = params.get("port");
  if (params.has("cert")) els.certHash.value = params.get("cert");
}

/** Decodes a lowercase-hex cert hash string into a Uint8Array digest. */
function hexToBytes(hex) {
  const clean = hex.trim().toLowerCase();
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(clean.substr(i * 2, 2), 16);
  }
  return out;
}

// A superseded attempt (a newer session token began) returns this so the
// reconnect controller does not treat it as a failure and reschedule — the
// newer flow owns recovery.
const SUPERSEDED = { ok: true };

// Opening one session. `manual` is true only for a user-initiated Connect:
// only then is the motion lease requested and control resumed. An automatic
// reconnect (`manual: false`) restores transport, telemetry, and video but
// NEVER re-requests motion authority — control stays suspended until the user
// explicitly reconnects. Returns `{ ok }` / `{ ok: false, failure }` for the
// reconnect controller.
async function connect({ manual } = { manual: true }) {
  const host = els.host.value.trim();
  const port = els.port.value.trim();
  const certHashHex = els.certHash.value.trim();
  if (!host || !port || !certHashHex) {
    log("host, port, and cert hash are all required");
    return { ok: false, failure: { phase: "construct" } };
  }
  // A SHA-256 hash is exactly 32 bytes; anything else is a configuration
  // error (non-retryable), not something WebTransport should be handed.
  if (!/^[0-9a-fA-F]{64}$/.test(certHashHex)) {
    log("certificate hash must be exactly 64 hexadecimal characters (SHA-256)");
    return { ok: false, failure: { phase: "construct" } };
  }
  const url = `https://${host}:${port}/pilotage`;
  const certHash = hexToBytes(certHashHex);

  // The gate re-arm, release-settlement wait, live lease probe, and
  // post-grant decision run inside the production orchestration module,
  // which the lifecycle tests execute directly.
  const session = await negotiateSessionAuthority({
    manual,
    gate: controlGate,
    releases: releaseTracker,
    openAndBootstrap: (leaseProbe) =>
      openTransportSession({ url, certHash, certHashHex, manual, leaseProbe }),
    startControl: ({ transport, token }) => {
      const ok = startControlLoop(transport, token);
      if (ok) sendGimbalLeaseRequest(token);
      return ok;
    },
    controlUnavailable: ({ token }) => {
      transportSessions.runIfActive(token, () => {
        state.leaseGranted = false;
        sendLeaseRelease(token);
      });
    },
    releaseLease: ({ token }) => {
      transportSessions.runIfActive(token, () => {
        state.leaseGranted = false;
        sendLeaseRelease(token);
        els.gamepad.textContent =
          "control released — input lost during connect; press Connect to resume";
        log("input lost during connect — lease released immediately");
      });
    },
    telemetryOnly: (_session, wasManual) => {
      if (wasManual) {
        // A telemetry-only vehicle (e.g. the Aviate adapter, ADR-0018)
        // advertises no controllable scopes; sending control frames
        // anyway would only generate a 30 Hz stream of rejections.
        log("no control lease granted; viewer is telemetry/video only");
      } else {
        log("reconnected (telemetry/video); press Connect to resume control");
      }
    },
  });
  return session.result;
}

/** Transport construction, handshake, and bootstrap for one session
 *  attempt. Resolves to `{ completed, leaseGranted, transport, token,
 *  result }`; `result` is connect()'s return contract for the reconnect
 *  controller (SUPERSEDED, ok, or a classified failure). */
async function openTransportSession({ url, certHash, certHashHex, manual, leaseProbe }) {
  let transport;
  try {
    transport = new WebTransport(url, {
      serverCertificateHashes: [{ algorithm: "sha-256", value: certHash }],
    });
  } catch (error) {
    log(`WebTransport creation failed: ${error}`);
    return { completed: false, result: { ok: false, failure: { phase: "construct" } } };
  }
  const token = transportSessions.begin(transport);
  transportSessions.runIfActive(token, () => {
    state.transport = transport;
    state.sessionWriter = null;
    state.sessionId = 0;
    state.generation = 0n;
    state.sequence = 0;
    state.gimbalGeneration = 0n;
    state.gimbalSequence = 0;
    state.gimbalLeaseGranted = false;
    state.prevGimbalReset = false;
    state.gimbalStreaming = false;
    state.prevArmInputs = new Set();
    state.pendingReset = false;
    state.pendingFpvToggle = false;
    state.connected = false;
    state.leaseGranted = false;
    state.skippedVideoFrames = 0;
    resetVideoDiagnostics();
    retireSessionPresentation("connecting");
    const mode = manual ? "requesting control" : "auto-reconnect, telemetry/video only";
    log(`connecting to ${url} (${mode}) pinned to cert hash ${certHashHex.slice(0, 16)}...`);
  });

  transport.closed.then(
    () => handleTransportClosed(token, null),
    (error) => handleTransportClosed(token, error),
  );

  try {
    await transport.ready;
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    log("WebTransport session ready");

    const bidi = await transport.createBidirectionalStream();
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    const writer = bidi.writable.getWriter();
    const reader = bidi.readable.getReader();
    if (!transportSessions.trackWriter(token, writer)) return { completed: false, result: SUPERSEDED };
    if (!transportSessions.trackReader(token, reader)) return { completed: false, result: SUPERSEDED };

    if (!(await sendClientHello(writer, token))) return { completed: false, result: SUPERSEDED };
    // Only a manual connect requests the motion lease; an auto-reconnect
    // completes bootstrap at ServerWelcome, leaving control suspended.
    const bootstrap = await runBootstrapReader({
      reader,
      decode: decodeLengthDelimitedEnvelope,
      isActive: () => transportSessions.isActive(token),
      onMessage: (decoded) => handleBootstrapMessage(decoded, token),
      // The orchestration module's live probe, evaluated at the
      // ServerWelcome moment immediately before the one LeaseRequest
      // emission: a blur latched during ANY await of this connect
      // suppresses the request entirely.
      requestLease: leaseProbe,
      sendLeaseRequest: () => sendLeaseRequest(writer, token),
    });
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    if (!bootstrap.completed) {
      // An untyped stream end is RETRYABLE wherever it lands in the
      // handshake: the host's Close carries no wire payload today, so a
      // pre-welcome EOF is indistinguishable from an ordinary early
      // transport drop, and classifying it as a rejection would
      // permanently stop recovery after a network blip. Only a TYPED
      // protocol rejection may set `bootstrapRejected` (none exists on
      // the wire yet — tracked in the reconnection issue).
      throw new Error(
        bootstrap.welcomed
          ? "bootstrap stream closed after welcome, before it completed"
          : "bootstrap stream closed before ServerWelcome",
      );
    }

    // Negotiation is the lifecycle boundary for measurement ordering. The
    // token prevents readers from the replaced transport from reaching this
    // newly empty ingress even if their promises settle later.
    instruments.ingress = newSimulatorAvionicsIngress();
    instruments.fcState = new FcStateTracker();
    setTelemetrySessionState(els, "awaiting");
    state.connected = true;
    // Bootstrap is proven good only here — reset the reconnect backoff now, not
    // at transport.ready (which precedes bootstrap).
    reconnect.notifyBootstrapComplete();
    state.sessionWriter = writer;
    runSessionStreamReader(reader, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`session stream reader stopped: ${error}`));
    });
    acceptIncomingUniStreams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`incoming uni-stream accept loop error: ${error}`));
    });
    readTelemetryDatagrams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`telemetry reader stopped: ${error}`));
    });
    return {
      completed: true,
      leaseGranted: state.leaseGranted,
      transport,
      token,
      result: { ok: true },
    };
  } catch (error) {
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    state.connected = false;
    state.transport = null;
    retireSessionPresentation("failed");
    log(`connect failed: ${error}`);
    transportSessions.close(token);
    // "rejected" (non-retryable) is reserved for a TYPED protocol
    // rejection or authenticated close code; an untyped failure — EOF,
    // reset, timeout — is always a retryable transport drop.
    const phase = error && error.bootstrapRejected ? "rejected" : "transport";
    return { completed: false, result: { ok: false, failure: { phase } } };
  }
}

function handleTransportClosed(token, error) {
  if (!transportSessions.isActive(token)) return;
  state.connected = false;
  state.transport = null;
  state.sessionWriter = null;
  // An unacknowledged release rides down with the transport; the host
  // watchdog covers it from here.
  releaseTracker.abandon();
  retireSessionPresentation(error === null ? "disconnected" : "failed");
  log(error === null ? "WebTransport session closed" : `WebTransport session errored: ${error}`);
  transportSessions.retire(token);
  // The drop was not user-initiated (there is no disconnect control); recover
  // the transport if the user still wants a session. A clean or errored close
  // is treated as a transient transport drop — retried with capped, jittered
  // backoff. Control is NOT resumed by the reconnect (see connect: manual).
  reconnect.notifyDropped({ phase: "transport" });
}

// Input-loss gate (CTRL-04): blur LATCHES loss synchronously so a
// blur/refocus between control-loop ticks can never be missed; only an
// explicit new connect re-arms it.
const controlGate = createControlGate({ isFocused: () => document.hasFocus() });

// One in-flight explicit lease release and its acknowledgement; an
// immediate reconnect waits (bounded) for settlement so it cannot race
// into an AlreadyHeld denial. The host watchdog stays the backup.
const releaseTracker = createReleaseTracker();

/** Auto-recovery for an unexpectedly dropped session: reconnects the transport
 *  (telemetry/video) with capped, jittered backoff, and never re-requests
 *  motion authority. Uses real timers, page visibility, and session state. */
const reconnect = createReconnectController({
  connect,
  schedule: (delayMs, cb) => setTimeout(cb, delayMs),
  cancel: (handle) => clearTimeout(handle),
  isVisible: () => document.visibilityState === "visible",
  isActive: () => transportSessions.active !== null,
  random: () => Math.random(),
  log,
});

/** Writes a length-delimited `ClientHello` envelope onto the bootstrap bidi stream. */
async function sendClientHello(writer, token) {
  if (!transportSessions.isActive(token)) return false;
  const hello = encodeClientHelloEnvelope({
    protocolVersion: 1,
    clientName: "pilotage-web-viewer",
    joinToken: new Uint8Array(0),
  });
  await writer.write(lengthDelimit(hello));
  if (!transportSessions.isActive(token)) return false;
  log("sent ClientHello");
  return true;
}

/** Writes a length-delimited `LeaseRequest` for the motion scope. */
async function sendLeaseRequest(writer, token) {
  if (!transportSessions.isActive(token)) return false;
  const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope: MOTION_SCOPE });
  await writer.write(lengthDelimit(request));
  if (!transportSessions.isActive(token)) return false;
  log(`sent LeaseRequest for ${MOTION_SCOPE}`);
  return true;
}

/** Requests the gimbal-scope lease on the reliable session stream, AFTER
 *  bootstrap completed: the bootstrap reader completes on the first
 *  LeaseResponse and discards its remaining buffer, so a second request
 *  in flight during the handshake could lose (or split) its response.
 *  Requested only once control has started; denial is non-fatal — flight
 *  continues, the client just never emits gimbal frames. */
function sendGimbalLeaseRequest(token) {
  const writer = state.sessionWriter;
  if (!writer || !transportSessions.isActive(token)) return;
  const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope: GIMBAL_SCOPE });
  writer.write(lengthDelimit(request)).catch(() => {
    // A dying reliable stream ends the session; nothing to do here.
  });
  log(`sent LeaseRequest for ${GIMBAL_SCOPE}`);
}

/** Sends a `LeaseRelease` for the motion scope on the reliable session
 *  stream and starts the bounded wait for the host's acknowledgement. */
function sendLeaseRelease(token) {
  const writer = state.sessionWriter;
  if (!writer || !transportSessions.isActive(token)) return;
  if (state.gimbalLeaseGranted) {
    // Fire-and-forget: the gimbal scope has no motion authority, so its
    // release rides best-effort alongside the tracked motion release
    // (the host watchdog covers a lost one).
    state.gimbalLeaseGranted = false;
    const gimbalRelease = encodeLeaseReleaseEnvelope({
      vehicleId: VEHICLE_ID,
      scope: GIMBAL_SCOPE,
    });
    writer.write(lengthDelimit(gimbalRelease)).catch(() => {});
    log("sent LeaseRelease for " + GIMBAL_SCOPE);
  }
  if (releaseTracker.isPending()) return; // one release per loss; the ack settles it
  const release = encodeLeaseReleaseEnvelope({ vehicleId: VEHICLE_ID, scope: MOTION_SCOPE });
  const settled = releaseTracker.begin();
  settled.then((outcome) => {
    transportSessions.runIfActive(token, () => log(`lease release ${outcome}`));
  });
  writer.write(lengthDelimit(release)).catch(() => {
    // The reliable stream refused the write: the transport is dying, and
    // the host watchdog will do the release.
    releaseTracker.abandon();
  });
  log("sent LeaseRelease for " + MOTION_SCOPE);
}

/** Reads the session (bootstrap) stream after the handshake completes:
 *  the host's `LeaseReleased` acknowledgement arrives here. */
async function runSessionStreamReader(reader, token) {
  let pending = new Uint8Array(0);
  for (;;) {
    const { value, done } = await reader.read();
    if (!transportSessions.isActive(token) || done) return;
    pending = appendBytes(pending, value);
    for (;;) {
      const decoded = decodeLengthDelimitedEnvelope(pending);
      if (!decoded) break;
      pending = pending.subarray(decoded.consumed);
      if (decoded.kind === "LeaseReleased") {
        // Validate the acknowledgement before it settles anything: an
        // ack for another vehicle or scope proves nothing about ours.
        const m = decoded.message;
        if (Number(m.vehicleId) === VEHICLE_ID && m.scope === MOTION_SCOPE) {
          releaseTracker.acknowledge();
          log(`LeaseReleased: released=${m.released} generation=${m.generation}`);
        } else if (Number(m.vehicleId) === VEHICLE_ID && m.scope === GIMBAL_SCOPE) {
          log(`LeaseReleased[gimbal]: released=${m.released} generation=${m.generation}`);
        } else {
          log(`ignoring LeaseReleased for vehicle=${m.vehicleId} scope=${m.scope}`);
        }
      }
      if (decoded.kind === "LeaseResponse" && decoded.message.scope === GIMBAL_SCOPE) {
        // The gimbal lease is requested after bootstrap, so its response
        // arrives here rather than in the bootstrap reader.
        const m = decoded.message;
        state.gimbalGeneration = BigInt(m.generation || 0);
        state.gimbalLeaseGranted = !!m.granted;
        log(`LeaseResponse[gimbal]: granted=${m.granted} generation=${m.generation}`);
        if (!m.granted) {
          log(`gimbal lease denied (reason ${m.reason}); flight control is unaffected`);
        }
      }
    }
  }
}

/** Prefixes an already-encoded `Envelope` with a protobuf varint byte-length, matching `encode_length_delimited` on the host. */
function lengthDelimit(envelopeBytes) {
  const prefix = [];
  let v = envelopeBytes.length;
  for (;;) {
    let byte = v & 0x7f;
    v >>>= 7;
    if (v !== 0) {
      prefix.push(byte | 0x80);
    } else {
      prefix.push(byte);
      break;
    }
  }
  const out = new Uint8Array(prefix.length + envelopeBytes.length);
  out.set(prefix, 0);
  out.set(envelopeBytes, prefix.length);
  return out;
}


function handleBootstrapMessage(decoded, token) {
  if (!transportSessions.isActive(token)) return;
  if (decoded.kind === "ServerWelcome") {
    state.sessionId = decoded.message.sessionId;
    log(`ServerWelcome: session=${decoded.message.sessionId} principal=${decoded.message.principalId}`);
  } else if (decoded.kind === "LeaseResponse") {
    // Bootstrap requests only the motion lease, but route by scope
    // anyway: a response for any other scope must not overwrite the
    // motion generation.
    if (decoded.message.scope && decoded.message.scope !== MOTION_SCOPE) {
      log(`ignoring bootstrap LeaseResponse for scope=${decoded.message.scope}`);
      return;
    }
    state.generation = BigInt(decoded.message.generation || 0);
    state.leaseGranted = !!decoded.message.granted;
    log(`LeaseResponse: granted=${decoded.message.granted} generation=${decoded.message.generation}`);
    if (!decoded.message.granted) {
      els.overlay.textContent = `lease denied (reason ${decoded.message.reason})`;
    }
  }
}

function appendBytes(existing, incoming) {
  const out = new Uint8Array(existing.length + incoming.length);
  out.set(existing, 0);
  out.set(incoming, existing.length);
  return out;
}

/** Coalesces uni-stream read failures into at most one log line per interval
 * (after the first, which logs immediately): a host that resets a stalled
 * frame's stream (ADR video deadline) surfaces a WebTransportError here every
 * frame, which after the first is expected, not per-frame news. */
function noteStreamReadFailure(error) {
  streamReadFailures.count += 1;
  const nowMs = performance.now();
  if (
    !shouldLogReadFailure(
      nowMs,
      streamReadFailures.lastLoggedMs,
      STREAM_READ_FAILURE_LOG_INTERVAL_MS,
    )
  ) {
    return;
  }
  streamReadFailures.lastLoggedMs = nowMs;
  log(`uni stream read failed: ${error} (${streamReadFailures.count} total this session)`);
}

/** Runs the one incoming-uni-stream accept loop for this session. An individual
 * received stream failing loses only that frame and the loop keeps accepting;
 * the collection stream itself closing or erroring is terminal for the whole
 * WebTransport session (a re-acquired reader is handed the same stored error),
 * so it is surfaced once as a session failure and never reacquired. */
async function acceptIncomingUniStreams(transport, token) {
  await runIncomingStreamAcceptLoop(transport.incomingUnidirectionalStreams, {
    isActive: () => transportSessions.isActive(token),
    handleStream: (stream) => readOneUniStream(stream, token),
    onStreamFailure: (error) =>
      transportSessions.runIfActive(token, () => noteStreamReadFailure(error)),
    onCollectionTerminal: (error) =>
      transportSessions.runIfActive(token, () => handleTransportClosed(token, error)),
    trackReader: (reader) => transportSessions.trackReader(token, reader),
    untrackReader: (reader) => transportSessions.untrackReader(token, reader),
  });
}

/** Drains one uni stream to completion, buffering bytes, reading the kind tag, then dispatching. */
async function readOneUniStream(stream, token) {
  if (!transportSessions.isActive(token)) return;
  const reader = stream.getReader();
  if (!transportSessions.trackReader(token, reader)) return;
  let buf = new Uint8Array(0);
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (!transportSessions.isActive(token)) return;
      if (value) buf = appendBytes(buf, value);
      if (done) break;
    }
    if (buf.length === 0) return;
    const kind = buf[0];
    const body = buf.subarray(1);
    if (kind === STREAM_KIND_AUTHORITY) {
      dispatchAuthorityStream(body, token);
    } else if (kind === STREAM_KIND_VIDEO_V2) {
      await renderVideoFrameV2(body, token);
    } else if (kind === STREAM_KIND_VIDEO) {
      await renderVideoFrame(body, token);
    } else {
      log(`unrecognized uni stream kind tag 0x${kind.toString(16)}`);
    }
  } finally {
    transportSessions.untrackReader(token, reader);
  }
}

/** The dedicated authority-events stream is opened once at connection start and may carry several length-delimited envelopes over the stream's lifetime; decode every complete one buffered. */
function dispatchAuthorityStream(body, token) {
  if (!transportSessions.isActive(token)) return;
  let pending = body;
  for (;;) {
    const decoded = decodeLengthDelimitedEnvelope(pending);
    if (!decoded) return;
    pending = pending.subarray(decoded.consumed);
    if (decoded.kind === "AuthorityEvent") {
      els.overlay.textContent = `authority: ${decoded.message.arm}`;
      log(`authority event: ${decoded.message.arm}`);
    }
  }
}

// The source_id (0 = onboard FPV, 1 = chase) routes a frame to its canvas. An
// unknown source_id is counted and logged, never a hard failure, so a host
// streaming a source this viewer lacks degrades gracefully. Codec dispatch is
// separate: "MJPG" paints via createImageBitmap; "H264" routes to WebCodecs.
const FOURCC_MJPEG = "MJPG";
const SOURCE_FPV = 0;
const SOURCE_CHASE = 1;
const VIDEO_TARGETS = {
  [SOURCE_FPV]: { canvas: els.canvas, ctx },
  [SOURCE_CHASE]: { canvas: els.chaseCanvas, ctx: chaseCtx },
};

// Per-source H.264 decoders, each bound to the transport session that built it
// (H264DecoderRegistry owns that lifetime — see video-h264.js). A source that
// only ever carries MJPEG never constructs one. The decoder's output callback
// tests its own session's liveness, so a reconnect's fresh decoder supersedes
// any retired one. The registry's ownership table lives in the instrument
// wasm, so construction must wait for that module to initialize — constructing
// it here would trap on the uninitialized wasm binding and take the whole
// viewer module down with it. Until the wasm is ready (or if it fails to
// load), the H.264 path skips frames visibly; MJPEG is unaffected.
let h264Registry = null;

function buildH264Registry() {
  return new H264DecoderRegistry((target, token) =>
    new H264CanvasDecoder(target, {
      log: (message) => log(message),
      isActive: () => transportSessions.isActive(token),
    }),
  );
}

/** Decodes one MJPEG payload and blits it to `target`, resizing its canvas to
 *  the frame. Session-token checked around the async decode so a frame decoded
 *  after teardown is dropped. */
async function paintJpeg(payload, target, token) {
  const bitmap = await createImageBitmap(new Blob([payload], { type: "image/jpeg" }));
  if (!transportSessions.isActive(token)) {
    bitmap.close();
    return;
  }
  const { canvas, ctx: targetCtx } = target;
  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
  }
  targetCtx.drawImage(bitmap, 0, 0);
  bitmap.close();
}

/** Routes an admitted frame's payload to the decoder for its codec, counting
 *  and logging a skip for a codec this viewer cannot decode. MJPEG paints
 *  directly; H.264 (Annex-B) routes to a per-source WebCodecs decoder, which
 *  fails closed when WebCodecs or the stream's profile is unavailable. */
async function paintByCodec(fourcc, payload, sourceId, target, token) {
  if (fourcc === FOURCC_MJPEG) {
    await paintJpeg(payload, target, token);
    return;
  }
  if (fourcc === FOURCC_H264) {
    if (!h264Registry) {
      state.skippedVideoFrames += 1;
      if (!state.h264UnavailableLogged) {
        state.h264UnavailableLogged = true;
        log("H.264 decode unavailable (instrument wasm not loaded); skipping H.264 frames silently");
      }
      return;
    }
    h264Registry.for(sourceId, target, token).decode(payload);
    return;
  }
  state.skippedVideoFrames += 1;
  log(`unknown video codec FourCC "${fourcc}" for source ${sourceId}; skipping frame (${state.skippedVideoFrames} skipped total)`);
}

/** Resolves a frame's canvas target by source id, counting/logging a skip for
 *  an unknown source. Returns the target, or `null` to skip. */
function videoTargetFor(sourceId) {
  const target = VIDEO_TARGETS[sourceId];
  if (!target) {
    state.skippedVideoFrames += 1;
    log(`unknown video source_id ${sourceId}; skipping frame (${state.skippedVideoFrames} skipped total)`);
    return null;
  }
  return target;
}

// v1 video body `[source_id][fourcc][u32 LE len][payload]` (ADR-0016), retained
// so a host that has not adopted the v2 capture-identity framing still renders.
async function renderVideoFrame(body, token) {
  if (!transportSessions.isActive(token)) return;
  if (body.length < 9) return;
  const sourceId = body[0];
  const fourcc = String.fromCharCode(body[1], body[2], body[3], body[4]);
  const view = new DataView(body.buffer, body.byteOffset + 5, 4);
  const len = view.getUint32(0, true);
  const payload = body.subarray(9, 9 + len);
  if (payload.length !== len) {
    log(`video frame length mismatch: declared ${len}, got ${payload.length}`);
    return;
  }
  const target = videoTargetFor(sourceId);
  if (target) await paintByCodec(fourcc, payload, sourceId, target, token);
}

// v2 video body: a capture-identity header, then `[fourcc][u32 LE len][payload]`
// (ADR-0020). The capture identity gates the frame through `videoIdentity`: a
// duplicate, reordered, stale-epoch, or wrong-camera frame is dropped and never
// blitted, so a replayed frame cannot displace a newer one or refresh its age.
// Association runs ONLY on a frame the tracker accepted (via
// `associateIfAccepted`), so a replay can never associate fresh. It finds the
// aircraft snapshot corresponding to the frame's CAPTURE time and passes the
// recognized calibrations for the frame's camera (from the hash-verified
// artifact, ADR-0021) to the gate. The verdict is a diagnostic surface only;
// no conformal overlay is drawn. Live, the FPV calibration now resolves, but
// the verdict still stays not-ready because the Gazebo rover publishes planar
// pose rather than an avionics snapshot to associate against, and Aviate's
// video clock mapping is unavailable — honestly.
// Per-source conformal-readiness, so a persistent state (e.g. Aviate's
// mapping-unavailable) is logged once on transition instead of every frame.
// `undefined` = never logged; `null` = last logged ready; a string = last
// logged not-ready reason. Reset per session (resetVideoDiagnostics).
const videoReadinessLog = new Map();

// Coalesces uni-stream read failures: under the host's per-frame stall/reset
// regime a reset surfaces as a WebTransportError on the frame's stream, which
// after the first is expected and must not spam one line per frame.
// `lastLoggedMs === null` means never logged, so the first failure logs at once.
const streamReadFailures = { count: 0, lastLoggedMs: null };
const STREAM_READ_FAILURE_LOG_INTERVAL_MS = 2000;

function resetVideoDiagnostics() {
  videoReadinessLog.clear();
  streamReadFailures.count = 0;
  streamReadFailures.lastLoggedMs = null;
}

/** Logs a per-source conformal-readiness change once, never per frame. */
function logVideoReadiness(sourceId, association) {
  const transition = readinessTransition(
    videoReadinessLog.get(sourceId),
    association.ready,
    association.reason,
  );
  if (!transition) return;
  videoReadinessLog.set(sourceId, transition.state);
  log(`video source ${sourceId} ${transition.message}`);
}

async function renderVideoFrameV2(body, token) {
  if (!transportSessions.isActive(token)) return;
  // Decode + capture-identity contract validation happen in wasm, from the
  // host's own wire definitions; `decoded` carries the meta (u64 fields as
  // BigInt, u32 as Number — exactly the kinds the gate checks), the codec, the
  // payload's position in `body`, and a typed `fault` when the header violates
  // the encoder contract. A structurally malformed body decodes to null.
  const decoded = decodeVideoFrameV2(body);
  if (!decoded) {
    state.skippedVideoFrames += 1;
    log(`malformed v2 video frame; skipping (${state.skippedVideoFrames} skipped total)`);
    return;
  }
  const { meta, fourcc, payloadOffset, payloadLen, fault } = decoded;
  if (fault) {
    state.droppedIdentityFrames += 1;
    log(
      `video frame rejected (${fault.field}:${fault.rule}) for source ${meta.sourceId} ` +
        `seq ${meta.sequence}; dropped (${state.droppedIdentityFrames} identity drops total)`,
    );
    return;
  }
  const target = videoTargetFor(meta.sourceId);
  if (!target) return;
  const payload = body.subarray(payloadOffset, payloadOffset + payloadLen);
  // The calibration effective-window check is genuine wall-clock (a calibration
  // is valid for a real-time period), unlike the capture-time association.
  const nowUnixNs = BigInt(Date.now()) * 1_000_000n;
  const recognizedCalibrations = calibrationRegistry.recognizedFor(meta.cameraId, nowUnixNs);
  const { admit, association } = associateIfAccepted(videoIdentity, snapshotHistory, meta, {
    recognizedCalibrations,
  });
  if (!admit.accepted) {
    state.droppedIdentityFrames += 1;
    log(
      `video frame not admitted (${admit.reason}) for source ${meta.sourceId} ` +
        `seq ${meta.sequence}; dropped (${state.droppedIdentityFrames} identity drops total)`,
    );
    return;
  }
  if (admit.discontinuity) {
    log(`video source ${meta.sourceId} capture discontinuity: fresh epoch/incarnation/calibration`);
    // A discontinuity is a GOP boundary an H.264 decoder cannot span; drop this
    // source's decoder so the next keyframe reconfigures a fresh one (MJPEG: no-op).
    h264Registry?.reset(meta.sourceId);
  }
  state.lastAssociation = association;
  logVideoReadiness(meta.sourceId, association);
  await paintByCodec(fourcc, payload, meta.sourceId, target, token);
}

/** Reads telemetry-fast datagrams (bare Envelope, TelemetrySample arm) forever, updating the pose overlay. */
async function readTelemetryDatagrams(transport, token) {
  if (!transportSessions.isActive(token)) return;
  const reader = transport.datagrams.readable.getReader();
  if (!transportSessions.trackReader(token, reader)) return;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (!transportSessions.isActive(token)) return;
      if (done) return;
      const decoded = decodeDatagramEnvelope(value);
      if (decoded.kind === "TelemetrySample") {
        const t = decoded.message;
        if (t.avionics) {
          instruments.ingress.ingest(t, performance.now());
          // Feed the association ring from the ACCEPTED snapshot the ingress
          // exposes (not the raw wire sample), anchored on the kinematics group:
          // position is what registers world features for a conformal overlay.
          // A snapshot without kinematics supplies no spatial anchor and is not
          // observed, so association stays fail-closed for it.
          const accepted = instruments.ingress.snapshot(performance.now());
          if (accepted.kinematics) {
            snapshotHistory.observe(accepted.kinematics.stamp);
          }
        }
        if (t.vehicleId !== VEHICLE_ID) continue;
        els.telemetry.textContent = formatTelemetrySummary(
          t,
          instruments.fcState.observe(t.fcState ?? null, performance.now()),
        );
      } else if (decoded.kind === "Pong") {
        // RTT probing is out of scope for this demo viewer; ignored.
      } else if (decoded.kind === "FrameRejected") {
        log(`control frame rejected (reason ${decoded.message.reason})`);
      }
    }
  } finally {
    transportSessions.untrackReader(token, reader);
  }
}

// ---- keyboard -> control frame datagrams -----------------------------------

const DRIVE_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "w",
  "a",
  "s",
  "d",
  "W",
  "A",
  "S",
  "D",
  "Enter",
  "Backspace",
]);

// Keys are stored raw (letters lower-cased) so rover and flight modes can
// map WASD and the arrows independently.
function canonicalKey(key) {
  return key.length === 1 ? key.toLowerCase() : key;
}
window.addEventListener("keydown", (event) => {
  if (DRIVE_KEYS.has(event.key)) {
    state.keys.add(canonicalKey(event.key));
    event.preventDefault();
  }
});
window.addEventListener("keyup", (event) => {
  if (DRIVE_KEYS.has(event.key)) {
    state.keys.delete(canonicalKey(event.key));
    event.preventDefault();
  }
});

/** Maps current key state to [throttle, yaw] axis values in [-1.0, 1.0]. */
function axesFromKeys() {
  const k = (key) => (state.keys.has(key) ? 1 : 0);
  const throttle = k("ArrowUp") + k("w") - k("ArrowDown") - k("s");
  const yaw = k("ArrowRight") + k("d") - k("ArrowLeft") - k("a");
  return [Math.max(-1, Math.min(1, throttle)), Math.max(-1, Math.min(1, yaw))];
}

/** Keyboard fallback for flight modes: W/S = climb/descend, A/D = yaw,
 *  arrows = forward/back + left/right translation. */
function flightAxesFromKeys() {
  const k = (key) => (state.keys.has(key) ? 1 : 0);
  return {
    roll: k("ArrowRight") - k("ArrowLeft"),
    pitch: k("ArrowUp") - k("ArrowDown"),
    throttle: k("w") - k("s"),
    yaw: k("d") - k("a"),
  };
}

// Per-controller FPV mapping profiles. EdgeTX radios (RadioMaster Pocket) output
// the "Classic Joystick" report in AETR channel order over 8 HID axes, and
// EdgeTX maps CH1->axis0, CH2->axis1, CH3->axis2, CH4->axis3, so:
//   axis 0 = Aileron (roll), 1 = Elevator (pitch), 2 = Throttle, 3 = Rudder (yaw).
// The FPV convention drives with throttle for speed and rudder for yaw (both the
// left stick in Mode 2). Throttle does not self-center — it holds its position
// like an aircraft throttle — so centering it stops the vehicle.
// Field of a profile: forwardAxis/turnAxis (HID axis index), forwardSign/turnSign
// (+1 or -1), deadzone. A `match(id)` picks the profile from the gamepad id.
const CONTROLLER_PROFILES = [
  {
    name: "RadioMaster Pocket (EdgeTX, FPV AETR)",
    match: (id) => /radiomaster|pocket|1209/i.test(id),
    forwardAxis: 2, // throttle
    forwardSign: 1, // stick up = forward, center = stop, down = reverse
    turnAxis: 3, // rudder
    turnSign: -1, // rudder right = turn right (negative yaw-rate in Gazebo)
  },
  {
    // PS4 (DualShock 4) / PS5 (DualSense) map to the browser "Standard Gamepad"
    // layout: axis 0 = left stick X, 1 = left stick Y, 2 = right stick X,
    // 3 = right stick Y, with stick-up reported as negative. Both sticks
    // self-center, so releasing stops the vehicle. Drive from the left stick
    // (throttle = vertical, yaw = horizontal), matching the FPV convention.
    name: "PS4/PS5 pad (Standard Gamepad)",
    match: (id) => /054c|dualshock|dualsense|wireless controller|standard gamepad/i.test(id),
    forwardAxis: 1, // left stick vertical
    forwardSign: -1, // stick up (negative) = forward
    turnAxis: 0, // left stick horizontal
    turnSign: -1, // stick right (positive) = turn right (negative yaw-rate)
    deadzone: 0.06,
    standard: true, // W3C Standard Gamepad layout: flight schemes apply
  },
];

// Flight-mode stick schemes for Standard Gamepad pads. Both
// command the identical velocity control law — only stick assignment
// differs. Browser reports stick-up as negative, hence the -1 signs.
//   pilot  = RC Mode 2 (camera-drone default): left = climb+yaw,
//            right = translate.
//   cruise = game-native: left = translate, right X = yaw,
//            R2/L2 analog triggers = climb/descend.
// DualSense standard mapping: axes 0/1 left X/Y, 2/3 right X/Y;
// buttons 6/7 = L2/R2 analog, 8 = create/share, 9 = options.
const FLIGHT_SCHEMES = {
  "quad-pilot": (raw) => ({
    throttle: -raw(1),
    yaw: raw(0),
    pitch: -raw(3),
    roll: raw(2),
    label: "PILOT (Mode 2): L=climb/yaw R=move",
  }),
  "quad-cruise": (raw, buttons) => ({
    pitch: -raw(1),
    roll: raw(0),
    yaw: raw(2),
    throttle: (buttons[7]?.value ?? 0) - (buttons[6]?.value ?? 0),
    label: "CRUISE: L=move RX=yaw R2/L2=climb",
  }),
  // FPV: same Mode-2 stick geometry as PILOT, but the adapter is in
  // attitude mode — right stick commands tilt ANGLES, throttle is
  // direct collective around hover. Toggle with the FPV button.
  fpv: (raw) => ({
    throttle: -raw(1),
    yaw: raw(0),
    pitch: -raw(3),
    roll: raw(2),
    label: "FPV: R=tilt angle L=thrust/yaw",
  }),
};
// Gamepad buttons that fire arm/disarm edges: options (9) arms,
// create/share (8) disarms — deliberately away from the face buttons.
const PAD_ARM_BUTTON = 9;
const PAD_DISARM_BUTTON = 8;

/** Maps a Standard-Gamepad pad to flight axes under the given scheme;
 *  non-standard pads (EdgeTX radios) use their AETR axes directly, which
 *  is true RC Mode 2 on a real radio. */
function flightAxesFromGamepad(pad, profile, mode) {
  const rawAt = (i) => (i >= 0 && i < pad.axes.length ? pad.axes[i] : 0);
  const clamp = (v) => Math.max(-1, Math.min(1, v));
  const shaped = stickShaper(profile);
  const raw = (i) => shaped(rawAt(i));
  if (profile.standard) {
    const scheme = FLIGHT_SCHEMES[mode] ?? FLIGHT_SCHEMES["quad-pilot"];
    const a = scheme(raw, pad.buttons ?? []);
    return {
      roll: clamp(a.roll),
      pitch: clamp(a.pitch),
      throttle: clamp(a.throttle),
      yaw: clamp(a.yaw),
      label: a.label,
    };
  }
  // EdgeTX AETR HID order: 0 roll, 1 pitch, 2 throttle, 3 yaw.
  return {
    roll: raw(0),
    pitch: -raw(1),
    throttle: raw(2),
    yaw: -raw(3),
    label: "radio AETR (Mode 2)",
  };
}

/** The arm/disarm inputs held right now, from gamepad buttons and keys: Enter
 *  arms, Backspace disarms; pad options (9) arms, create (8) disarms. */
function currentArmInputs(pad) {
  return pressedArmInputs({
    padArm: !!pad?.buttons?.[PAD_ARM_BUTTON]?.pressed,
    padDisarm: !!pad?.buttons?.[PAD_DISARM_BUTTON]?.pressed,
    keyArm: state.keys.has("Enter"),
    keyDisarm: state.keys.has("Backspace"),
  });
}

/** One-shot arm/disarm edges: only inputs newly pressed since last tick fire.
 *  `state.prevArmInputs` is primed to the held set at control start, so a
 *  button held across a reconnect does not read as a fresh command. */
function collectArmEdges(pad) {
  const pressedNow = currentArmInputs(pad);
  const edges = [];
  for (const which of risingArmEdges(pressedNow, state.prevArmInputs)) {
    const arm = which.endsWith("-arm");
    edges.push([arm ? BUTTON_ARM : BUTTON_DISARM, BUTTON_EDGE_PRESSED]);
    els.overlay.textContent = arm ? "ARM sent" : "DISARM sent";
    log(arm ? "arm command sent" : "disarm command sent");
  }
  state.prevArmInputs = pressedNow;
  return edges;
}
// Fallback for any other gamepad: drive from the left stick's self-centering
// vertical/horizontal axes so the vehicle stops when the stick is released.
const GENERIC_PROFILE = {
  name: "generic gamepad",
  forwardAxis: 1,
  forwardSign: -1, // browsers report stick-up as negative
  turnAxis: 0,
  turnSign: -1, // stick right (positive) = turn right (negative yaw-rate in Gazebo)
  deadzone: 0.1,
};

// User overrides for the active profile, taken from URL query params today; this
// is the seam a future in-app remapping UI plugs into. Any of ?fwd= ?turn=
// ?fwdsign= ?turnsign= ?deadzone= replaces the corresponding profile field.
function overrideProfile(profile) {
  const q = new URLSearchParams(window.location.search);
  const intParam = (name, fallback) => {
    const v = Number.parseInt(q.get(name) ?? "", 10);
    return Number.isInteger(v) ? v : fallback;
  };
  const numParam = (name, fallback) => {
    const v = Number.parseFloat(q.get(name) ?? "");
    return Number.isFinite(v) ? v : fallback;
  };
  const signParam = (name, fallback) => (intParam(name, fallback) < 0 ? -1 : 1);
  return {
    name: q.has("fwd") || q.has("turn") ? `${profile.name} (overridden)` : profile.name,
    forwardAxis: intParam("fwd", profile.forwardAxis),
    forwardSign: signParam("fwdsign", profile.forwardSign),
    turnAxis: intParam("turn", profile.turnAxis),
    turnSign: signParam("turnsign", profile.turnSign),
    deadzone: numParam("deadzone", profile.deadzone),
    standard: profile.standard === true,
  };
}

/** Returns the first connected gamepad that matches a known profile, else any
 *  connected gamepad, else null. A gamepad is exposed to the page only after the
 *  user moves a stick or presses a button once. */
function activeGamepad() {
  const pads = (navigator.getGamepads && navigator.getGamepads()) || [];
  let firstConnected = null;
  for (const pad of pads) {
    if (!pad || !pad.connected) continue;
    firstConnected = firstConnected || pad;
    if (CONTROLLER_PROFILES.some((p) => p.match(pad.id))) return pad;
  }
  return firstConnected;
}

/** Picks the FPV profile for a gamepad id and applies user overrides. */
function profileFor(id) {
  const base = CONTROLLER_PROFILES.find((p) => p.match(id)) || GENERIC_PROFILE;
  return overrideProfile(base);
}

/** Maps a gamepad's axes to [throttle, yaw] in [-1.0, 1.0] under `profile`. */
function axesFromGamepad(pad, profile) {
  const raw = (i) => (i >= 0 && i < pad.axes.length ? pad.axes[i] : 0);
  const clamp = (v) => Math.max(-1, Math.min(1, v));
  const deadzone = (v) => (Math.abs(v) < profile.deadzone ? 0 : v);
  const throttle = clamp(deadzone(profile.forwardSign * raw(profile.forwardAxis)));
  const yaw = clamp(deadzone(profile.turnSign * raw(profile.turnAxis)));
  return [throttle, yaw];
}

/** Updates the gamepad readout so the active profile and axis-to-control mapping
 *  are visible while driving (and easy to re-map with the ?fwd=/?turn= overrides). */
function updateGamepadReadout(pad, profile, throttle, yaw) {
  if (!pad) {
    els.gamepad.textContent = "gamepad: none (move a stick to detect) — using keyboard";
    return;
  }
  const axes = Array.from(pad.axes, (v) => v.toFixed(2)).join(", ");
  els.gamepad.textContent =
    `gamepad: ${pad.id} [${profile.name}] | axes[${axes}] | ` +
    `throttle=${throttle.toFixed(2)} (axis ${profile.forwardAxis}) ` +
    `yaw=${yaw.toFixed(2)} (axis ${profile.turnAxis})`;
}

/** Shows the live 4-axis flight mapping and stick values. */
function updateFlightReadout(pad, f) {
  const src = pad ? `${pad.id.slice(0, 24)} [${f.label}]` : "keyboard (WS=climb AD=yaw arrows=move)";
  els.gamepad.textContent =
    `flight: ${src} | roll=${f.roll.toFixed(2)} pitch=${f.pitch.toFixed(2)} ` +
    `climb=${f.throttle.toFixed(2)} yaw=${f.yaw.toFixed(2)} | arm: Options/Enter, disarm: Create/Backspace`;
}

/** Neutral (zeroed) control axes for a flight mode: rover carries only
 *  throttle/yaw, the flight modes carry roll/pitch/throttle/yaw. */
function neutralAxesFor(mode) {
  return mode === "rover"
    ? [
        [AXIS_THROTTLE, 0],
        [AXIS_YAW, 0],
      ]
    : [
        [AXIS_ROLL, 0],
        [AXIS_PITCH, 0],
        [AXIS_THROTTLE, 0],
        [AXIS_YAW, 0],
      ];
}

/** Sends one control-fast datagram at `CONTROL_HZ`, carrying the latest key-derived axes (superseded samples are droppable, ADR-0011). */
function startControlLoop(transport, token) {
  const started = startDatagramControl({
    datagrams: transport.datagrams,
    lifecycle: transportSessions,
    token,
    run: (writer) => runControlLoop(writer, token),
    onError: (error) => log(`control loop stopped: ${error}`),
  });
  if (!started.ok) {
    const detail =
      started.reason === "send-stream-unavailable"
        ? "no datagram send stream"
        : "datagram writer acquisition failed";
    els.gamepad.textContent = `control unavailable — ${detail}`;
    els.overlay.textContent = "control unavailable";
    log(`control unavailable: ${detail}`);
  }
  return started.ok;
}

/** Runs the control-fast send loop with an acquired, session-owned writer. */
async function runControlLoop(writer, token) {
  // Prime the arm-edge baseline to whatever is held right now, so a button held
  // down as control starts (e.g. across a reconnect) is not read as a fresh
  // arm/disarm on the first tick.
  state.prevArmInputs = currentArmInputs(activeGamepad());
  const intervalMs = 1000 / CONTROL_HZ;
  // Self-paced async loop rather than setInterval: it awaits the writer's
  // backpressure signal (`ready`) before each send, so datagrams never queue up
  // in the WritableStream and get flushed in a burst with stale `sampled_at`
  // (which the host rejects as too old, ADR-0009). `sampled_at` is stamped right
  // before the write, after `ready`, so it reflects the real send moment.
  while (transportSessions.isActive(token) && state.connected) {
      try {
        await writer.ready;
      } catch {
        return; // writer closed (session ended)
      }
      if (!transportSessions.isActive(token) || !state.connected) return;
      // A connected gamepad drives under its profile; the keyboard is the
      // fallback when none is present. The readout shows the live mapping.
      const mode = els.flightMode ? els.flightMode.value : "rover";
      if (!controlGate.mayPublish()) {
        // Input loss RELINQUISHES control authority. The latch is
        // absolute: NO frame — not even a neutral — is sent under this
        // generation, because the explicit LeaseRelease (sent by the
        // blur handler the instant the latch set; re-attempted here for
        // the polled-unfocused fallback) makes the host fence the
        // generation and drive the vehicle to its link-loss policy. A
        // client-side neutral would both violate the latch invariant and
        // refresh the very setpoint freshness the silence watchdog (the
        // independent backup) is watching.
        state.prevArmInputs = new Set();
        sendLeaseRelease(token);
        els.gamepad.textContent =
          "control released — input lost; press Connect to resume control";
        log("input lost — control authority released (host acknowledges or watchdog covers)");
        return;
      }
      const pad = activeGamepad();
      const profile = pad ? profileFor(pad.id) : null;
      let axes;
      let edges = [];
      if (mode === "rover") {
        // Ground vehicles (the Gazebo yard world) accept only the
        // throttle/yaw pair; extra axes would be rejected as unknown.
        const [throttle, yaw] = pad ? axesFromGamepad(pad, profile) : axesFromKeys();
        updateGamepadReadout(pad, profile, throttle, yaw);
        axes = [
          [AXIS_THROTTLE, throttle],
          [AXIS_YAW, yaw],
        ];
      } else {
        const gimbalHeld = gimbalModifierHeld(pad, profile);
        const flightView = gimbalHeld ? gimbalMaskedView(pad) : pad;
        const f = pad ? flightAxesFromGamepad(flightView, profile, mode) : flightAxesFromKeys();
        if (gimbalHeld) {
          const g = gimbalAxesFromGamepad(pad, profile);
          els.gamepad.textContent =
            `GIMBAL (LT held): RS pitch=${g.pitch.toFixed(2)} yaw=${g.yaw.toFixed(2)} | ` +
            "LT descend inhibited — release LT for flight | R3 recenters";
        } else {
          updateFlightReadout(pad, f);
        }
        edges = collectArmEdges(pad);
        if (state.pendingFpvToggle) {
          state.pendingFpvToggle = false;
          edges.push([BUTTON_FPV_TOGGLE, BUTTON_EDGE_PRESSED]);
        }
        if (state.pendingReset) {
          state.pendingReset = false;
          edges.push([BUTTON_RESET, BUTTON_EDGE_PRESSED]);
          log("simulation reset requested");
        }
        axes = [
          [AXIS_ROLL, f.roll],
          [AXIS_PITCH, f.pitch],
          [AXIS_THROTTLE, f.throttle],
          [AXIS_YAW, f.yaw],
        ];
      }
      // Re-check the latch immediately before the write: no frame may be
      // sent under this generation once input loss latched, regardless of
      // which await the latch interleaved with.
      if (!controlGate.mayPublish()) continue;
      state.sequence = (state.sequence + 1) >>> 0; // wraps at u32, matching the wire SequenceNum width.
      const envelope = encodeControlFrameEnvelope({
        sessionId: state.sessionId,
        vehicleId: VEHICLE_ID,
        scope: MOTION_SCOPE,
        generation: state.generation,
        sequence: state.sequence,
        sampledAtNanos: nowNanos(),
        profileRevision: 1,
        axes,
        edges,
      });
      writer.write(envelope).catch((error) => {
        transportSessions.runIfActive(token, () => log(`control datagram send failed: ${error}`));
      });
      maybeSendGimbalFrame(writer, token, mode, pad, profile);
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}

/** Emits one gimbal-scope frame EVERY tick while the lease is held,
 *  after the motion frame under the same publish gate: LT held streams
 *  the right stick's LOS rates, otherwise the frame carries zero rates.
 *  The continuous stream is the scope's liveness (ADR-0011), exactly
 *  like motion: a gimbal holder that goes quiet between LT presses
 *  would trip the host's holder-silence watchdog, whose link-loss
 *  policy is per-VEHICLE and would neutralize flight. R3 fires the
 *  recenter edge; active-demand shaping lives in `gimbalFramePlan`. */
function maybeSendGimbalFrame(writer, token, mode, pad, profile) {
  if (mode === "rover" || !state.gimbalLeaseGranted) return;
  const resetHeld = !!pad?.buttons?.[PAD_GIMBAL_RESET]?.pressed;
  const resetEdge = resetHeld && !state.prevGimbalReset;
  state.prevGimbalReset = resetHeld;
  const plan = gimbalFramePlan({
    held: gimbalModifierHeld(pad, profile),
    resetEdge,
    streaming: state.gimbalStreaming,
    rates: pad ? gimbalAxesFromGamepad(pad, profile) : { pitch: 0, yaw: 0 },
  }) ?? { rates: { pitch: 0, yaw: 0 }, recenter: false, streaming: false };
  if (plan.recenter) log("gimbal recenter requested (R3)");
  state.gimbalSequence = (state.gimbalSequence + 1) >>> 0;
  const envelope = encodeControlFrameEnvelope({
    sessionId: state.sessionId,
    vehicleId: VEHICLE_ID,
    scope: GIMBAL_SCOPE,
    generation: state.gimbalGeneration,
    sequence: state.gimbalSequence,
    sampledAtNanos: nowNanos(),
    profileRevision: 1,
    axes: [
      [AXIS_PITCH, plan.rates.pitch],
      [AXIS_YAW, plan.rates.yaw],
    ],
    edges: plan.recenter ? [[GIMBAL_BUTTON_NEUTRAL, BUTTON_EDGE_PRESSED]] : [],
  });
  writer.write(envelope).catch((error) => {
    transportSessions.runIfActive(token, () => log(`gimbal datagram send failed: ${error}`));
  });
  state.gimbalStreaming = plan.streaming;
}

// ---- instrument panels (ADR-0017) -------------------------------------------

// DYN-01: turn-rate derivation is measurement-coherent — it advances
// only on a NEW accepted heading measurement identity and differences
// over the acquisition clock (see turn-derivation.js). Rendering
// cadence cannot inflate or zero the rate, and session retirement
// resets it below.
const turnDerivation = new TurnDerivation();

/** The explicit SIM heading declaration: local-NED yaw from the
 * published attitude quaternion, declared sim-local-true. Returns null
 * when no attitude estimate exists — no sample, never a zero heading. */
function declaredSimHeading(attitude) {
  const q = attitude?.quat;
  if (!q || ![q.w, q.x, q.y, q.z].every(Number.isFinite)) return null;
  const yaw = Math.atan2(2 * (q.w * q.z + q.x * q.y), 1 - 2 * (q.y * q.y + q.z * q.z));
  return { rad: yaw, reference: 2, ageMs: attitude.ageMs };
}


/** Maps the latest wire avionics estimate into the instrument state ABI
 * and draws both panels; runs on the display's own rAF cadence. Every
 * render result is honored: a validated frame is blitted, anything else
 * is covered by the failure page (no ignored results and no stale
 * imagery). */
function renderInstruments() {
  const mod = instruments.mod;
  if (!mod) {
    // Load/ABI/init faults are shared-backend failures: both panels show
    // the failure page. While still loading there is no prior imagery to
    // cover, so the canvases stay blank.
    if (instruments.moduleFault !== null) {
      coverInstrumentFailures(instruments.health, instrumentTargets());
    }
    return;
  }
  const snapshot = instruments.ingress.snapshot(performance.now());
  const attitude = snapshot.attitude;
  const kinematics = snapshot.kinematics;
  const validFlags = snapshot.validFlags;
  const coherence = {
    insufficient: 0,
    coherent: 1,
    "excessive-skew": 2,
  }[snapshot.coherence.status];
  // NAV-01: the feeder — not the display — declares the simulator's
  // heading explicitly. Local-NED yaw is computed HERE from the same
  // estimate the vehicle published and declared as sim-local-true
  // (reference 2); the display never derives heading from attitude on
  // its own, so removing this declaration flags HDG instead of
  // freezing a fabricated rose.
  const heading = declaredSimHeading(attitude);
  const dynamics = turnDerivation.update(
    heading === null ? NaN : heading.rad,
    heading === null ? NaN : heading.ageMs,
    attitude?.stamp ?? null,
  );
  const panelState = {
    attitude,
    kinematics,
    air: null, // no airspeed/baro sensor on Aviate's wire yet (ADR-0018): honest Missing.
    nav: null,
    wind: null,
    selections: { headingBugRad: 0, headingBugReference: 2 },
    heading,
    dynamics,
    quality: snapshot.quality,
    valid: {
      attitude: !!(validFlags & 1),
      rates: !!(validFlags & 2),
      position: !!(validFlags & 4),
      velocity: !!(validFlags & 8),
      heading: heading !== null && !!(validFlags & 1),
      turn: dynamics !== null && !!(validFlags & 1),
    },
    snapshot: { generation: snapshot.generation, coherence },
  };
  const nowMs = performance.now();
  renderInstrumentSet(mod, instruments.health, instrumentTargets(), panelState, nowMs);
}

/** Liveness check on its own timer so a stalled render loop still gets
 * its stale frame covered; skipped while the module is absent (a load
 * fault already shows its own page, and before load there is no imagery
 * to cover). */
function watchdogTick() {
  if (!instruments.mod) return;
  tickInstrumentSet(instruments.health, instrumentTargets(), performance.now());
}

async function startInstruments() {
  startDisplayLoop(
    (callback) => requestAnimationFrame(callback),
    () => renderInstruments(),
    () =>
      failInstrumentSet(
        instruments.health,
        instrumentTargets(),
        performance.now(),
        REASON.RENDER_TRAP,
      ),
  );
  try {
    // Revalidate the wasm on every load: this page is served statically with
    // no build step, so a rebuilt binary must not be masked by a heuristically
    // cached copy (a 304 keeps it cheap when unchanged).
    const wasmSource = await fetch("./instrument-runtime_bg.wasm", { cache: "no-cache" });
    instruments.mod = await loadInstruments(wasmSource);
    h264Registry = buildH264Registry();
    const nowMs = performance.now();
    for (const health of Object.values(instruments.health)) health.reset(nowMs);
    setInterval(watchdogTick, WATCHDOG_INTERVAL_MS);
    log("instrument panels ready (wasm loaded)");
  } catch (error) {
    instruments.moduleFault = error?.reason ?? REASON.WASM_LOAD;
    failInstrumentSet(
      instruments.health,
      instrumentTargets(),
      performance.now(),
      instruments.moduleFault,
    );
    log(`instrument panels unavailable (D-${instruments.moduleFault}): ${error} (run scripts/build-web-instruments.sh)`);
  }
}

window.addEventListener("pagehide", () => instruments.mod?.dispose(), { once: true });
applyUrlParams();
startInstruments();
document.getElementById("fpvBtn").addEventListener("click", () => {
  state.pendingFpvToggle = true;
});
document.getElementById("resetBtn").addEventListener("click", () => {
  state.pendingReset = true;
});
els.connectBtn.addEventListener("click", () => {
  // An explicit, user-initiated start: resets the reconnect backoff and is the
  // only path that requests control.
  reconnect.requestConnect();
});
// A launcher-pinned URL (host + port + cert all present, autoconnect=1)
// connects on load through the SAME explicit path as the button — the URL
// already states everything the click would add. Anything less than a
// fully pinned URL still requires the click. Runs after all wiring so the
// full connect stack (control gate, reconnect controller) is in place.
{
  const params = new URLSearchParams(window.location.search);
  if (
    params.get("autoconnect") === "1" &&
    els.host.value &&
    els.port.value &&
    els.certHash.value
  ) {
    reconnect.requestConnect();
  }
}

// Returning to a tab the browser had frozen is the moment to recover: the
// session likely idle-dropped while away, and only now can a reconnect succeed
// (a hidden tab would just drop again).
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") reconnect.notifyVisible();
});

// Losing window focus can swallow keyup events, leaving keys "stuck". Clear the
// keyboard and arm-edge state immediately on blur so input released off-window
// does not linger and a button still held when focus returns cannot edge.
window.addEventListener("blur", () => {
  // Latch FIRST, synchronously: the control loop may be parked on an
  // await, and a refocus before its next tick must not un-happen the
  // loss. Frames under the current generation end here — and the
  // explicit release starts HERE too, not at the loop's next tick, so a
  // Connect click racing the blur always finds the release in flight.
  controlGate.latchInputLoss();
  if (state.leaseGranted && transportSessions.active !== null) {
    sendLeaseRelease(transportSessions.active);
  }
  state.keys.clear();
  state.prevArmInputs = new Set();
});
