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
// kind-tag byte (0x01 authority-events, 0x02 one video frame); read
// telemetry-fast datagrams for live pose; send control-fast datagrams
// (bare Envelope, ControlFrame arm) from arrow/WASD key state.

import {
  encodeClientHelloEnvelope,
  encodeLeaseRequestEnvelope,
  encodeControlFrameEnvelope,
  decodeLengthDelimitedEnvelope,
  decodeBareEnvelope,
  STREAM_KIND_AUTHORITY,
  STREAM_KIND_VIDEO,
  BUTTON_EDGE_PRESSED,
} from "./wire.js";
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
import { formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";
import { AvionicsIngress, INCARNATION_POLICY } from "./telemetry-ingress.js";
import { TransportSessionLifecycle } from "./transport-session.js";

const VEHICLE_ID = 1n; // demo fixture: the single Gazebo vehicle this host serves.
const INSTRUMENT_SOURCE_ID = 1n; // explicit simulator adapter source; never first-packet selection.
// Aviate publishes attitude at 10 Hz and kinematics at 4 Hz. This simulator
// profile admits one kinematics period plus transport jitter; an aircraft
// profile must derive its own limit from the intended function.
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
  skippedVideoFrames: 0,
};
const transportSessions = new TransportSessionLifecycle();

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
  setTelemetrySessionState(els, phase);
}

const instruments = {
  mod: null,
  moduleFault: null,
  ingress: newSimulatorAvionicsIngress(),
  health: {
    [PANEL.PFD]: new PanelHealth(),
    [PANEL.HSI]: new PanelHealth(),
  },
};

// Browser watchdog cadence (simulator-only): a scheduling domain separate
// from requestAnimationFrame, so a stalled render loop still trips the
// liveness deadline and covers the stale frame.
const WATCHDOG_INTERVAL_MS = 250;

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

async function connect() {
  const host = els.host.value.trim();
  const port = els.port.value.trim();
  const certHashHex = els.certHash.value.trim();
  if (!host || !port || !certHashHex) {
    log("host, port, and cert hash are all required");
    return;
  }
  const url = `https://${host}:${port}/pilotage`;
  const certHash = hexToBytes(certHashHex);

  let transport;
  try {
    transport = new WebTransport(url, {
      serverCertificateHashes: [{ algorithm: "sha-256", value: certHash }],
    });
  } catch (error) {
    log(`WebTransport creation failed: ${error}`);
    return;
  }
  const token = transportSessions.begin(transport);
  transportSessions.runIfActive(token, () => {
    state.transport = transport;
    state.sessionId = 0;
    state.generation = 0n;
    state.sequence = 0;
    state.prevArmInputs = new Set();
    state.pendingReset = false;
    state.pendingFpvToggle = false;
    state.connected = false;
    state.leaseGranted = false;
    state.skippedVideoFrames = 0;
    retireSessionPresentation("connecting");
    log(`connecting to ${url} pinned to cert hash ${certHashHex.slice(0, 16)}...`);
  });

  transport.closed.then(
    () => handleTransportClosed(token, null),
    (error) => handleTransportClosed(token, error),
  );

  try {
    await transport.ready;
    if (!transportSessions.isActive(token)) return;
    log("WebTransport session ready");

    const bidi = await transport.createBidirectionalStream();
    if (!transportSessions.isActive(token)) return;
    const writer = bidi.writable.getWriter();
    const reader = bidi.readable.getReader();
    if (!transportSessions.trackWriter(token, writer)) return;
    if (!transportSessions.trackReader(token, reader)) return;

    if (!(await sendClientHello(writer, token))) return;
    const negotiated = await runBootstrapReader(reader, writer, token);
    if (!transportSessions.isActive(token)) return;
    if (!negotiated) throw new Error("bootstrap stream closed before LeaseResponse");

    // Negotiation is the lifecycle boundary for measurement ordering. The
    // token prevents readers from the replaced transport from reaching this
    // newly empty ingress even if their promises settle later.
    instruments.ingress = newSimulatorAvionicsIngress();
    setTelemetrySessionState(els, "awaiting");
    state.connected = true;
    acceptIncomingUniStreams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`uni stream accept failed: ${error}`));
    });
    readTelemetryDatagrams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`telemetry reader stopped: ${error}`));
    });
    if (state.leaseGranted) {
      startControlLoop(transport, token).catch((error) => {
        transportSessions.runIfActive(token, () => log(`control loop stopped: ${error}`));
      });
    } else {
      // A telemetry-only vehicle (e.g. the Aviate adapter, ADR-0018)
      // advertises no controllable scopes; sending control frames anyway
      // would only generate a 30 Hz stream of rejections.
      log("no control lease granted; viewer is telemetry/video only");
    }
  } catch (error) {
    if (!transportSessions.isActive(token)) return;
    state.connected = false;
    state.transport = null;
    retireSessionPresentation("failed");
    log(`connect failed: ${error}`);
    transportSessions.close(token);
  }
}

function handleTransportClosed(token, error) {
  if (!transportSessions.isActive(token)) return;
  state.connected = false;
  state.transport = null;
  retireSessionPresentation(error === null ? "disconnected" : "failed");
  log(error === null ? "WebTransport session closed" : `WebTransport session errored: ${error}`);
  transportSessions.retire(token);
}

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

/** Reads bootstrap frames until ServerWelcome and LeaseResponse establish the session. */
async function runBootstrapReader(reader, writer, token) {
  let pending = new Uint8Array(0);
  let sentLease = false;
  for (;;) {
    const { value, done } = await reader.read();
    if (!transportSessions.isActive(token)) return false;
    if (done) return false;
    pending = appendBytes(pending, value);
    for (;;) {
      const decoded = decodeLengthDelimitedEnvelope(pending);
      if (!decoded) break;
      pending = pending.subarray(decoded.consumed);
      handleBootstrapMessage(decoded, token);
      if (decoded.kind === "ServerWelcome" && !sentLease) {
        sentLease = true;
        if (!(await sendLeaseRequest(writer, token))) return false;
      }
      if (decoded.kind === "LeaseResponse") {
        return true;
      }
    }
  }
}

function handleBootstrapMessage(decoded, token) {
  if (!transportSessions.isActive(token)) return;
  if (decoded.kind === "ServerWelcome") {
    state.sessionId = decoded.message.sessionId;
    log(`ServerWelcome: session=${decoded.message.sessionId} principal=${decoded.message.principalId}`);
  } else if (decoded.kind === "LeaseResponse") {
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

/** Accepts every host-initiated uni stream and dispatches on its leading kind-tag byte. */
async function acceptIncomingUniStreams(transport, token) {
  if (!transportSessions.isActive(token)) return;
  const uniStreams = transport.incomingUnidirectionalStreams;
  const streamReader = uniStreams.getReader();
  if (!transportSessions.trackReader(token, streamReader)) return;
  try {
    for (;;) {
      const { value: stream, done } = await streamReader.read();
      if (!transportSessions.isActive(token)) return;
      if (done) return;
      readOneUniStream(stream, token).catch((error) => {
        transportSessions.runIfActive(token, () => log(`uni stream read failed: ${error}`));
      });
    }
  } finally {
    transportSessions.untrackReader(token, streamReader);
  }
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

// Video body is `[source_id: u8][fourcc: 4 bytes][u32 LE len][payload]` after
// the kind tag (ADR-0016; host stream_tag.rs `frame_video_payload`). The
// source_id (0 = onboard FPV, 1 = chase) routes the frame to its canvas. An
// unknown source_id or unknown FourCC is counted and logged, never a hard
// failure, so a host streaming a source or codec this viewer lacks degrades
// gracefully. Only "MJPG" is decoded here.
const FOURCC_MJPEG = "MJPG";
const SOURCE_FPV = 0;
const SOURCE_CHASE = 1;
const VIDEO_TARGETS = {
  [SOURCE_FPV]: { canvas: els.canvas, ctx },
  [SOURCE_CHASE]: { canvas: els.chaseCanvas, ctx: chaseCtx },
};

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
  const target = VIDEO_TARGETS[sourceId];
  if (!target) {
    state.skippedVideoFrames += 1;
    log(`unknown video source_id ${sourceId}; skipping frame (${state.skippedVideoFrames} skipped total)`);
    return;
  }
  if (fourcc !== FOURCC_MJPEG) {
    state.skippedVideoFrames += 1;
    log(`unknown video codec FourCC "${fourcc}" for source ${sourceId}; skipping frame (${state.skippedVideoFrames} skipped total)`);
    return;
  }
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
      const decoded = decodeBareEnvelope(value);
      if (decoded.kind === "TelemetrySample") {
        const t = decoded.message;
        if (t.avionics) {
          instruments.ingress.ingest(t, performance.now());
        }
        if (t.vehicleId !== VEHICLE_ID) continue;
        els.telemetry.textContent = formatTelemetrySummary(t);
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
  const dz = profile.deadzone ?? 0.1;
  // Cubic expo (50%): fine authority near center, full range at the
  // ends — half of the DJI feel; the uplink's slew limit is the other.
  const expo = (v) => 0.35 * v * v * v + 0.65 * v;
  const shaped = (v) => expo(clamp(Math.abs(v) < dz ? 0 : v));
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

/** One-shot arm/disarm edges from gamepad buttons and keys: Enter arms,
 *  Backspace disarms; pad options (9) arms, create (8) disarms. */
function collectArmEdges(pad) {
  const pressedNow = new Set();
  if (pad?.buttons?.[PAD_ARM_BUTTON]?.pressed) pressedNow.add("pad-arm");
  if (pad?.buttons?.[PAD_DISARM_BUTTON]?.pressed) pressedNow.add("pad-disarm");
  if (state.keys.has("Enter")) pressedNow.add("key-arm");
  if (state.keys.has("Backspace")) pressedNow.add("key-disarm");
  const edges = [];
  for (const which of pressedNow) {
    if (!state.prevArmInputs.has(which)) {
      const arm = which.endsWith("-arm");
      edges.push([arm ? BUTTON_ARM : BUTTON_DISARM, BUTTON_EDGE_PRESSED]);
      els.overlay.textContent = arm ? "ARM sent" : "DISARM sent";
      log(arm ? "arm command sent" : "disarm command sent");
    }
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

/** Sends one control-fast datagram at `CONTROL_HZ`, carrying the latest key-derived axes (superseded samples are droppable, ADR-0011). */
async function startControlLoop(transport, token) {
  if (!transportSessions.isActive(token)) return;
  const writer = transport.datagrams.writable.getWriter();
  if (!transportSessions.trackWriter(token, writer)) return;
  const intervalMs = 1000 / CONTROL_HZ;
  // Self-paced async loop rather than setInterval: it awaits the writer's
  // backpressure signal (`ready`) before each send, so datagrams never queue up
  // in the WritableStream and get flushed in a burst with stale `sampled_at`
  // (which the host rejects as too old, ADR-0009). `sampled_at` is stamped right
  // before the write, after `ready`, so it reflects the real send moment.
  try {
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
        const f = pad ? flightAxesFromGamepad(pad, profile, mode) : flightAxesFromKeys();
        updateFlightReadout(pad, f);
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
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  } finally {
    transportSessions.untrackWriter(token, writer);
  }
}

// ---- instrument panels (ADR-0017) -------------------------------------------

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
  const quality = Math.max(attitude?.quality ?? 0, kinematics?.quality ?? 0);
  const coherence = {
    insufficient: 0,
    coherent: 1,
    "excessive-skew": 2,
  }[snapshot.coherence.status];
  const panelState = {
    attitude,
    kinematics,
    air: null, // no airspeed/baro sensor on Aviate's wire yet (ADR-0018): honest Missing.
    nav: null,
    wind: null,
    selections: { headingBugRad: 0 },
    quality,
    valid: {
      attitude: !!(attitude?.validFlags & 1),
      rates: !!(attitude?.validFlags & 2),
      position: !!(kinematics?.validFlags & 4),
      velocity: !!(kinematics?.validFlags & 8),
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
    instruments.mod = await loadInstruments("./instrument-runtime_bg.wasm");
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
  void connect();
});
