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
} from "./wire.js";
import { loadInstruments, PANEL } from "./instruments.js";

const VEHICLE_ID = 1n; // demo fixture: the single Gazebo vehicle this host serves.
const MOTION_SCOPE = "vehicle.motion";
const CONTROL_HZ = 30; // continuous control send rate; superseded samples are droppable (ADR-0011).
const AXIS_THROTTLE = 2; // pilotage-input logical axis table: throttle = 2.
const AXIS_YAW = 3; // pilotage-input logical axis table: yaw = 3.

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
};
const ctx = els.canvas.getContext("2d");
const chaseCtx = els.chaseCanvas.getContext("2d");
const pfdCtx = els.pfd.getContext("2d");
const hsiCtx = els.hsi.getContext("2d");

/** Session-scoped mutable state the connect flow and background loops share. */
const state = {
  transport: null,
  sessionId: 0,
  generation: 0n,
  sequence: 0,
  startNanos: BigInt(Date.now()) * 1_000_000n, // arbitrary local monotonic-ish origin for sampled_at (ADR-0009: endpoint-local, never compared raw across endpoints).
  keys: new Set(),
  connected: false,
  leaseGranted: false,
  skippedVideoFrames: 0,
};

// Instrument panel state (ADR-0017/0018): the latest raw avionics
// estimate plus its receive stamp. Ingest only records; drawing happens
// on the display's own requestAnimationFrame cadence, so telemetry rate
// and frame rate stay decoupled.
const instruments = {
  mod: null,
  latest: null,
  receivedAtMs: null,
};

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

  log(`connecting to ${url} pinned to cert hash ${certHashHex.slice(0, 16)}...`);
  const transport = new WebTransport(url, {
    serverCertificateHashes: [{ algorithm: "sha-256", value: certHash }],
  });
  state.transport = transport;

  transport.closed
    .then(() => {
      state.connected = false;
      log("WebTransport session closed");
    })
    .catch((error) => {
      state.connected = false;
      log(`WebTransport session errored: ${error}`);
    });

  await transport.ready;
  log("WebTransport session ready");

  const bidi = await transport.createBidirectionalStream();
  const writer = bidi.writable.getWriter();
  const reader = bidi.readable.getReader();

  await sendClientHello(writer);
  await runBootstrapReader(reader, writer);

  state.connected = true;
  acceptIncomingUniStreams(transport);
  readTelemetryDatagrams(transport);
  if (state.leaseGranted) {
    startControlLoop(transport).catch((error) => log(`control loop stopped: ${error}`));
  } else {
    // A telemetry-only vehicle (e.g. the Aviate adapter, ADR-0018)
    // advertises no controllable scopes; sending control frames anyway
    // would only generate a 30 Hz stream of rejections.
    log("no control lease granted; viewer is telemetry/video only");
  }
}

/** Writes a length-delimited `ClientHello` envelope onto the bootstrap bidi stream. */
async function sendClientHello(writer) {
  const hello = encodeClientHelloEnvelope({
    protocolVersion: 1,
    clientName: "pilotage-web-viewer",
    joinToken: new Uint8Array(0),
  });
  await writer.write(lengthDelimit(hello));
  log("sent ClientHello");
}

/** Writes a length-delimited `LeaseRequest` for the motion scope. */
async function sendLeaseRequest(writer) {
  const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope: MOTION_SCOPE });
  await writer.write(lengthDelimit(request));
  log(`sent LeaseRequest for ${MOTION_SCOPE}`);
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

/** Reads bootstrap-stream frames until ServerWelcome and LeaseResponse are both seen, then keeps forwarding any later frames (Pong/FrameRejected) to the log. */
async function runBootstrapReader(reader, writer) {
  let pending = new Uint8Array(0);
  let sentLease = false;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    pending = appendBytes(pending, value);
    for (;;) {
      const decoded = decodeLengthDelimitedEnvelope(pending);
      if (!decoded) break;
      pending = pending.subarray(decoded.consumed);
      handleBootstrapMessage(decoded);
      if (decoded.kind === "ServerWelcome" && !sentLease) {
        sentLease = true;
        await sendLeaseRequest(writer);
      }
      if (decoded.kind === "LeaseResponse") {
        return; // bootstrap complete; later frames are drained by a background reader.
      }
    }
  }
}

function handleBootstrapMessage(decoded) {
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
async function acceptIncomingUniStreams(transport) {
  const uniStreams = transport.incomingUnidirectionalStreams;
  const streamReader = uniStreams.getReader();
  for (;;) {
    const { value: stream, done } = await streamReader.read();
    if (done) return;
    readOneUniStream(stream).catch((error) => log(`uni stream read failed: ${error}`));
  }
}

/** Drains one uni stream to completion, buffering bytes, reading the kind tag, then dispatching. */
async function readOneUniStream(stream) {
  const reader = stream.getReader();
  let buf = new Uint8Array(0);
  for (;;) {
    const { value, done } = await reader.read();
    if (value) buf = appendBytes(buf, value);
    if (done) break;
  }
  if (buf.length === 0) return;
  const kind = buf[0];
  const body = buf.subarray(1);
  if (kind === STREAM_KIND_AUTHORITY) {
    dispatchAuthorityStream(body);
  } else if (kind === STREAM_KIND_VIDEO) {
    await renderVideoFrame(body);
  } else {
    log(`unrecognized uni stream kind tag 0x${kind.toString(16)}`);
  }
}

/** The dedicated authority-events stream is opened once at connection start and may carry several length-delimited envelopes over the stream's lifetime; decode every complete one buffered. */
function dispatchAuthorityStream(body) {
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

async function renderVideoFrame(body) {
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
  const { canvas, ctx: targetCtx } = target;
  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
  }
  targetCtx.drawImage(bitmap, 0, 0);
  bitmap.close();
}

/** Reads telemetry-fast datagrams (bare Envelope, TelemetrySample arm) forever, updating the pose overlay. */
async function readTelemetryDatagrams(transport) {
  const reader = transport.datagrams.readable.getReader();
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    const decoded = decodeBareEnvelope(value);
    if (decoded.kind === "TelemetrySample") {
      const t = decoded.message;
      els.telemetry.textContent =
        `pose x=${t.xM.toFixed(2)}m y=${t.yM.toFixed(2)}m heading=${t.headingRad.toFixed(2)}rad` +
        ` | v=${t.linearXMps.toFixed(2)}m/s w=${t.angularRadS.toFixed(2)}rad/s`;
      if (t.avionics) {
        instruments.latest = t.avionics;
        instruments.receivedAtMs = performance.now();
      }
    } else if (decoded.kind === "Pong") {
      // RTT probing is out of scope for this demo viewer; ignored.
    } else if (decoded.kind === "FrameRejected") {
      log(`control frame rejected (reason ${decoded.message.reason})`);
    }
  }
}

// ---- keyboard -> control frame datagrams -----------------------------------

const DRIVE_KEYS = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "w", "a", "s", "d", "W", "A", "S", "D"]);

window.addEventListener("keydown", (event) => {
  if (DRIVE_KEYS.has(event.key)) {
    state.keys.add(normalizeKey(event.key));
    event.preventDefault();
  }
});
window.addEventListener("keyup", (event) => {
  if (DRIVE_KEYS.has(event.key)) {
    state.keys.delete(normalizeKey(event.key));
    event.preventDefault();
  }
});

function normalizeKey(key) {
  const map = { w: "ArrowUp", s: "ArrowDown", a: "ArrowLeft", d: "ArrowRight" };
  return map[key.toLowerCase()] || key;
}

/** Maps current key state to [throttle, yaw] axis values in [-1.0, 1.0]. */
function axesFromKeys() {
  let throttle = 0;
  let yaw = 0;
  if (state.keys.has("ArrowUp")) throttle += 1;
  if (state.keys.has("ArrowDown")) throttle -= 1;
  if (state.keys.has("ArrowLeft")) yaw -= 1;
  if (state.keys.has("ArrowRight")) yaw += 1;
  return [throttle, yaw];
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
    deadzone: 0.12, // thumbsticks drift more than radio gimbals
    deadzone: 0.06,
  },
];
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

/** Sends one control-fast datagram at `CONTROL_HZ`, carrying the latest key-derived axes (superseded samples are droppable, ADR-0011). */
async function startControlLoop(transport) {
  const writer = transport.datagrams.writable.getWriter();
  const intervalMs = 1000 / CONTROL_HZ;
  // Self-paced async loop rather than setInterval: it awaits the writer's
  // backpressure signal (`ready`) before each send, so datagrams never queue up
  // in the WritableStream and get flushed in a burst with stale `sampled_at`
  // (which the host rejects as too old, ADR-0009). `sampled_at` is stamped right
  // before the write, after `ready`, so it reflects the real send moment.
  while (state.connected) {
    try {
      await writer.ready;
    } catch {
      return; // writer closed (session ended)
    }
    if (!state.connected) return;
    // A connected gamepad drives under its FPV profile; the keyboard is the
    // fallback when none is present. The readout shows the live mapping either way.
    const pad = activeGamepad();
    const profile = pad ? profileFor(pad.id) : null;
    const [throttle, yaw] = pad ? axesFromGamepad(pad, profile) : axesFromKeys();
    updateGamepadReadout(pad, profile, throttle, yaw);
    state.sequence = (state.sequence + 1) >>> 0; // wraps at u32, matching the wire SequenceNum width.
    const envelope = encodeControlFrameEnvelope({
      sessionId: state.sessionId,
      vehicleId: VEHICLE_ID,
      scope: MOTION_SCOPE,
      generation: state.generation,
      sequence: state.sequence,
      sampledAtNanos: nowNanos(),
      profileRevision: 1,
      axes: [
        [AXIS_THROTTLE, throttle],
        [AXIS_YAW, yaw],
      ],
    });
    writer.write(envelope).catch((error) => log(`control datagram send failed: ${error}`));
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}

// ---- instrument panels (ADR-0017) -------------------------------------------

/** Maps the latest wire avionics estimate into the instrument state ABI
 * and draws both panels; runs on the display's own rAF cadence. */
function renderInstruments() {
  const mod = instruments.mod;
  if (!mod) return;
  const a = instruments.latest;
  const ageMs = a ? performance.now() - instruments.receivedAtMs : NaN;
  mod.writeState({
    attitude: a ? { quat: a.quat, rates: a.rates, ageMs } : null,
    kinematics: a ? { posNed: a.posNed, velNed: a.velNed, ageMs } : null,
    air: null, // no airspeed/baro sensor on Aviate's wire yet (ADR-0018): honest Missing.
    nav: null,
    wind: null,
    selections: { headingBugRad: 0 },
    quality: a ? a.quality : 0,
    valid: a
      ? {
          attitude: !!(a.validFlags & 1),
          rates: !!(a.validFlags & 2),
          position: !!(a.validFlags & 4),
          velocity: !!(a.validFlags & 8),
        }
      : {},
  });
  mod.renderTo(pfdCtx, PANEL.PFD, els.pfd.width, els.pfd.height);
  mod.renderTo(hsiCtx, PANEL.HSI, els.hsi.width, els.hsi.height);
}

async function startInstruments() {
  try {
    instruments.mod = await loadInstruments("./instruments.wasm");
    const loop = () => {
      renderInstruments();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
    log("instrument panels ready (wasm loaded)");
  } catch (error) {
    log(`instrument panels unavailable: ${error} (run scripts/build-web-instruments.sh)`);
  }
}

applyUrlParams();
startInstruments();
els.connectBtn.addEventListener("click", () => {
  connect().catch((error) => log(`connect failed: ${error}`));
});
