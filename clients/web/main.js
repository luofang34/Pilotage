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
  canvas: document.getElementById("video"),
};
const ctx = els.canvas.getContext("2d");

/** Session-scoped mutable state the connect flow and background loops share. */
const state = {
  transport: null,
  sessionId: 0,
  generation: 0n,
  sequence: 0,
  startNanos: BigInt(Date.now()) * 1_000_000n, // arbitrary local monotonic-ish origin for sampled_at (ADR-0009: endpoint-local, never compared raw across endpoints).
  keys: new Set(),
  connected: false,
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
  startControlLoop(transport);
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

// Video body is `[fourcc: 4 bytes][u32 LE len][payload]` after the kind tag
// (ADR-0016; host stream_tag.rs `frame_video_payload`). An unknown FourCC is
// skipped with a log line, never a hard failure, so a host streaming a codec
// this viewer lacks degrades gracefully. Only "MJPG" is decoded here.
const FOURCC_MJPEG = "MJPG";
async function renderVideoFrame(body) {
  if (body.length < 8) return;
  const fourcc = String.fromCharCode(body[0], body[1], body[2], body[3]);
  const view = new DataView(body.buffer, body.byteOffset + 4, 4);
  const len = view.getUint32(0, true);
  const payload = body.subarray(8, 8 + len);
  if (payload.length !== len) {
    log(`video frame length mismatch: declared ${len}, got ${payload.length}`);
    return;
  }
  if (fourcc !== FOURCC_MJPEG) {
    log(`unknown video codec FourCC "${fourcc}"; skipping frame`);
    return;
  }
  const bitmap = await createImageBitmap(new Blob([payload], { type: "image/jpeg" }));
  if (els.canvas.width !== bitmap.width || els.canvas.height !== bitmap.height) {
    els.canvas.width = bitmap.width;
    els.canvas.height = bitmap.height;
  }
  ctx.drawImage(bitmap, 0, 0);
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

/** Sends one control-fast datagram at `CONTROL_HZ`, carrying the latest key-derived axes (superseded samples are droppable, ADR-0011). */
function startControlLoop(transport) {
  const writer = transport.datagrams.writable.getWriter();
  const intervalMs = 1000 / CONTROL_HZ;
  setInterval(() => {
    if (!state.connected) return;
    const [throttle, yaw] = axesFromKeys();
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
  }, intervalMs);
}

applyUrlParams();
els.connectBtn.addEventListener("click", () => {
  connect().catch((error) => log(`connect failed: ${error}`));
});
