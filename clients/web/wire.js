// Minimal hand-rolled protobuf wire codec for the `pilotage.v1` schemas this
// demo viewer speaks (ADR-0014: protobuf is the wire-schema source of truth;
// this file is a by-hand JS mirror of a subset of it, since the browser has
// no generated bindings). Field numbers below are read directly from
// schemas/pilotage/v1/*.proto — see the comment above each builder/parser for
// the exact (message, field) pairs relied on.
//
// Wire layout this file implements (matches hosts/session-host exactly):
//   - Datagrams carry exactly one bare `Envelope` proto message, no length
//     prefix (ADR-0014, ADR-0005).
//   - The bootstrap bidi stream and every host-initiated uni stream carry
//     length-delimited `Envelope` messages: a protobuf varint byte-length
//     prefix followed by that many bytes of `Envelope` (ADR-0014). Every
//     host-initiated uni stream additionally leads with one raw kind-tag
//     byte before the first length-delimited envelope
//     (hosts/session-host/src/runtime/stream_tag.rs): 0x01 = authority-events
//     stream, 0x02 = one video frame
//     (`[source_id: u8][fourcc: 4 bytes][u32 LE len][jpeg]` after the tag,
//     ADR-0016, not an Envelope at all). source_id 0 = onboard FPV, 1 = chase.

export const STREAM_KIND_AUTHORITY = 0x01;
export const STREAM_KIND_VIDEO = 0x02;
// 0x03 = one video frame that leads with a capture-identity header before the
// codec-tagged, length-prefixed payload (ADR-0020;
// hosts/session-host/src/runtime/stream_tag.rs `frame_video_payload_v2`).
export const STREAM_KIND_VIDEO_V2 = 0x03;

// The `pilotage.v1` schema version this build produces and accepts
// (pilotage_protocol::convert::SCHEMA_VERSION mirrors this constant).
export const SCHEMA_VERSION = 1;

// Byte offsets of the fixed v2 capture-identity header (little-endian), a
// by-hand mirror of `frame_video_payload_v2` in
// hosts/session-host/src/runtime/stream_tag.rs. `PAYLOAD` is also the header
// length (source_id .. calibration_id) plus the 4-byte FourCC and 4-byte u32
// length prefix.
const V2_OFFSET = Object.freeze({
  sourceId: 0,
  sourceEpoch: 1,
  sourceIncarnation: 5,
  sequence: 21,
  captureTime: 25,
  captureClock: 33,
  mappingAvailable: 34,
  mappingTargetClock: 35,
  mappingOffset: 36,
  clockErrorBound: 44,
  receiveTime: 52,
  publicationTime: 60,
  cameraId: 68,
  calibrationId: 72,
  fourcc: 76,
  length: 80,
  payload: 84,
});

/**
 * Parses a v2 video-frame body: the bytes after the `STREAM_KIND_VIDEO_V2`
 * (0x03) kind tag. Returns `{ meta, fourcc, payload }`, where `meta` is the
 * capture identity and clock mapping (source identity/epoch/incarnation,
 * wrapping sequence, sim capture time and clock, the mapping to the
 * flight-state clock with its quantified error bound, host receive/publication
 * times, and camera/calibration identities), `fourcc` is the 4-char codec tag,
 * and `payload` is the encoded frame. Returns `null` if the body is shorter
 * than the fixed header or the declared payload length does not match.
 */
export function parseVideoFrameV2(body) {
  if (body.length < V2_OFFSET.payload) return null;
  const view = new DataView(body.buffer, body.byteOffset, body.length);
  const len = view.getUint32(V2_OFFSET.length, true);
  const payload = body.subarray(V2_OFFSET.payload, V2_OFFSET.payload + len);
  if (payload.length !== len) return null;
  // Reject a non-canonical mapping-availability octet (anything but 0/1) here,
  // before it is normalized to a bool, matching the Rust/wasm decoder: `=== 1`
  // alone would read 2 as "unavailable" and, with target clock 0, slip past the
  // identity gate.
  const mappingOctet = body[V2_OFFSET.mappingAvailable];
  if (mappingOctet > 1) return null;
  const fourcc = String.fromCharCode(
    body[V2_OFFSET.fourcc],
    body[V2_OFFSET.fourcc + 1],
    body[V2_OFFSET.fourcc + 2],
    body[V2_OFFSET.fourcc + 3],
  );
  const meta = {
    sourceId: body[V2_OFFSET.sourceId],
    sourceEpoch: view.getUint32(V2_OFFSET.sourceEpoch, true),
    sourceIncarnation: decodeIncarnation(
      body.subarray(V2_OFFSET.sourceIncarnation, V2_OFFSET.sourceIncarnation + 16),
    ),
    sequence: view.getUint32(V2_OFFSET.sequence, true),
    captureTimeNanos: view.getBigUint64(V2_OFFSET.captureTime, true),
    captureClock: body[V2_OFFSET.captureClock],
    mappingAvailable: mappingOctet === 1,
    mappingTargetClock: body[V2_OFFSET.mappingTargetClock],
    mappingOffsetNanos: view.getBigInt64(V2_OFFSET.mappingOffset, true),
    clockErrorBoundNanos: view.getBigUint64(V2_OFFSET.clockErrorBound, true),
    receiveTimeNanos: view.getBigUint64(V2_OFFSET.receiveTime, true),
    publicationTimeNanos: view.getBigUint64(V2_OFFSET.publicationTime, true),
    cameraId: view.getUint32(V2_OFFSET.cameraId, true),
    calibrationId: view.getUint32(V2_OFFSET.calibrationId, true),
  };
  return { meta, fourcc, payload };
}

// ---- varint + protobuf tag primitives -------------------------------------

/** Appends an unsigned LEB128 varint to `bytes` (protobuf's varint encoding). */
function writeVarint(bytes, value) {
  let v = BigInt(value);
  for (;;) {
    let byte = Number(v & 0x7fn);
    v >>= 7n;
    if (v !== 0n) {
      bytes.push(byte | 0x80);
    } else {
      bytes.push(byte);
      return;
    }
  }
}

/** Reads an unsigned LEB128 varint without losing uint64 precision. */
function readVarintBigInt(view, offset) {
  let result = 0n;
  let shift = 0n;
  let pos = offset;
  for (;;) {
    const byte = view[pos];
    pos += 1;
    result |= BigInt(byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return [result, pos];
    }
    shift += 7n;
  }
}

/** Reads a varint used as a protobuf tag or byte length. */
function readVarint(view, offset) {
  const [value, next] = readVarintBigInt(view, offset);
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error("protobuf tag/length exceeds the safe integer range");
  }
  return [Number(value), next];
}

/** Writes a protobuf field tag: (fieldNumber << 3) | wireType. */
function writeTag(bytes, fieldNumber, wireType) {
  writeVarint(bytes, (fieldNumber << 3) | wireType);
}

const WIRE_VARINT = 0;
const WIRE_64BIT = 1;
const WIRE_LEN = 2;
const WIRE_32BIT = 5;

/** Writes a varint-typed field, skipping it entirely when `value` is falsy/zero (proto3 default-omission). */
function fieldVarint(bytes, fieldNumber, value) {
  if (!value) return;
  writeTag(bytes, fieldNumber, WIRE_VARINT);
  writeVarint(bytes, value);
}

/** Writes a length-delimited (bytes/string/submessage) field, skipping it when
 *  the payload is empty (proto3 default-omission for optional fields). */
function fieldBytes(bytes, fieldNumber, payload) {
  if (!payload || payload.length === 0) return;
  writeTag(bytes, fieldNumber, WIRE_LEN);
  writeVarint(bytes, payload.length);
  for (const b of payload) bytes.push(b);
}

/** Writes a required message-typed field, emitting it even when the submessage
 *  is empty (i.e. all its scalars are the proto3 default). The host's
 *  wire->domain conversion requires these submessages present, so — like the
 *  Rust encoder — an all-zero id (e.g. session 0) must still appear on the
 *  wire as an empty submessage, not be omitted. */
function fieldMessage(bytes, fieldNumber, payload) {
  writeTag(bytes, fieldNumber, WIRE_LEN);
  writeVarint(bytes, payload.length);
  for (const b of payload) bytes.push(b);
}

/** Writes a UTF-8 string field. */
function fieldString(bytes, fieldNumber, str) {
  if (!str) return;
  fieldBytes(bytes, fieldNumber, Array.from(new TextEncoder().encode(str)));
}

/** Writes a fixed 32-bit float field (proto3 `float`). */
function fieldFloat(bytes, fieldNumber, value) {
  if (!value) return;
  writeTag(bytes, fieldNumber, WIRE_32BIT);
  const buf = new ArrayBuffer(4);
  new DataView(buf).setFloat32(0, value, true);
  for (const b of new Uint8Array(buf)) bytes.push(b);
}

/** Concatenates an array of byte arrays into a plain array of numbers. */
function concatBytes(parts) {
  const out = [];
  for (const part of parts) for (const b of part) out.push(b);
  return out;
}

// ---- common.proto message builders ----------------------------------------
// message VehicleId { uint64 value = 1; }
// message ScopeId { string value = 1; }
// message SessionId { uint64 value = 1; }
// message Generation { uint64 value = 1; }

function encodeVehicleId(value) {
  const bytes = [];
  fieldVarint(bytes, 1, value);
  return bytes;
}

function encodeScopeId(value) {
  const bytes = [];
  fieldString(bytes, 1, value);
  return bytes;
}

// ---- session.proto: ClientHello (field numbers 1,2,3) ----------------------
// message ClientHello {
//   uint32 protocol_version = 1;
//   string client_name = 2;
//   bytes join_token = 3;
// }
function encodeClientHello({ protocolVersion, clientName, joinToken }) {
  const bytes = [];
  fieldVarint(bytes, 1, protocolVersion);
  fieldString(bytes, 2, clientName);
  fieldBytes(bytes, 3, joinToken || []);
  return bytes;
}

// ---- session.proto: LeaseRequest (field numbers 1,2) -----------------------
// message LeaseRequest { VehicleId vehicle = 1; ScopeId scope = 2; }
function encodeLeaseRequest({ vehicleId, scope }) {
  const bytes = [];
  fieldBytes(bytes, 1, encodeVehicleId(vehicleId));
  fieldBytes(bytes, 2, encodeScopeId(scope));
  return bytes;
}

// ---- envelope.proto: Envelope oneof arm numbers ----------------------------
// (envelope.proto): schema_version = 1; oneof payload: control_frame=2,
// authority_event=3, telemetry_sample=4, host_capabilities=5, client_hello=6,
// server_welcome=7, lease_request=8, lease_response=9, ping=10, pong=11,
// frame_rejected=12, lease_release=13, lease_released=14.
const ENVELOPE_FIELD = {
  schemaVersion: 1,
  controlFrame: 2,
  authorityEvent: 3,
  telemetrySample: 4,
  hostCapabilities: 5,
  clientHello: 6,
  serverWelcome: 7,
  leaseRequest: 8,
  leaseResponse: 9,
  ping: 10,
  pong: 11,
  frameRejected: 12,
  leaseRelease: 13,
  leaseReleased: 14,
  linkLossCleared: 15,
  profileActivation: 16,
  controlActionResult: 17,
};

/** Wraps an already-encoded payload submessage bytes in an `Envelope`. */
function encodeEnvelope(payloadFieldNumber, payloadBytes) {
  const bytes = [];
  fieldVarint(bytes, ENVELOPE_FIELD.schemaVersion, SCHEMA_VERSION);
  fieldBytes(bytes, payloadFieldNumber, payloadBytes);
  return bytes;
}

/** Encodes a `ClientHello` as a bare `Envelope` (payload field 6). */
export function encodeClientHelloEnvelope(hello) {
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.clientHello, encodeClientHello(hello)));
}

/** Encodes a `LeaseRequest` as a bare `Envelope` (payload field 8). */
export function encodeLeaseRequestEnvelope(request) {
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.leaseRequest, encodeLeaseRequest(request)));
}

// ---- session.proto: LeaseRelease (field numbers 1,2) -----------------------
// message LeaseRelease { VehicleId vehicle = 1; ScopeId scope = 2; }
function encodeLeaseRelease({ vehicleId, scope }) {
  const bytes = [];
  fieldBytes(bytes, 1, encodeVehicleId(vehicleId));
  fieldBytes(bytes, 2, encodeScopeId(scope));
  return bytes;
}

/** Encodes a `LeaseRelease` as a bare `Envelope` (payload field 13). */
export function encodeLeaseReleaseEnvelope(release) {
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.leaseRelease, encodeLeaseRelease(release)));
}

// ---- control.proto: ControlFrame (field numbers per schema) ----------------
// message AxisSample { uint32 axis_id = 1; float value = 2; }
// message ControlPayload { repeated AxisSample axes = 1; repeated ButtonEdgeSample edges = 2; }
// message ControlFrame {
//   SessionId session = 1; VehicleId vehicle = 2; ScopeId scope = 3;
//   Generation generation = 4; SequenceNum sequence = 5;
//   MonoTimestamp sampled_at = 6; uint32 profile_revision = 7;
//   ControlPayload payload = 8;
// }
function encodeAxisSample(axisId, value) {
  const bytes = [];
  fieldVarint(bytes, 1, axisId);
  fieldFloat(bytes, 2, value);
  return bytes;
}

function encodeControlPayload(axes, edges) {
  const bytes = [];
  for (const [axisId, value] of axes) {
    // Every reported axis must appear on the wire even when fully
    // neutral (axis 0 at value 0 encodes to an EMPTY submessage): the
    // host's full-coverage neutral checks — link-loss recovery and
    // reset-latch clearance — require every declared axis REPORTED,
    // and a sample omitted by proto3 default-skipping reads as "this
    // axis was never demonstrated".
    fieldMessage(bytes, 1, encodeAxisSample(axisId, value));
  }
  for (const [buttonId, edge] of edges ?? []) {
    fieldBytes(bytes, 2, encodeButtonEdgeSample(buttonId, edge));
  }
  return bytes;
}

// control.proto ButtonEdgeSample: button_id=1, edge=2
// (ButtonEdge enum: 1 = pressed, 2 = released).
export const BUTTON_EDGE_PRESSED = 1;

function encodeButtonEdgeSample(buttonId, edge) {
  const bytes = [];
  fieldVarint(bytes, 1, buttonId);
  fieldVarint(bytes, 2, edge);
  return bytes;
}

// control.proto typed-command enums (CTRL-01).
export const REFERENCE_FRAME_BODY_FRD = 1;
export const CONTROL_ACTION = {
  arm: 1,
  disarm: 2,
  modeRequest: 3,
  gimbalRecenter: 4,
  simReset: 5,
};
export const MODE_TARGET = {
  cameraVelocity: 1,
  fpvDirect: 2,
  hold: 3,
  return: 4,
};

// ControlIntent { oneof family: velocity=1 ... gimbal_rate=5 }
// VelocityIntent { frame=1, vx=2, vy=3, vz=4, yaw_rate=5 }
function encodeVelocityIntent({ vx, vy, vz, yawRate }) {
  const bytes = [];
  fieldVarint(bytes, 1, REFERENCE_FRAME_BODY_FRD);
  fieldFloat(bytes, 2, vx);
  fieldFloat(bytes, 3, vy);
  fieldFloat(bytes, 4, vz);
  fieldFloat(bytes, 5, yawRate);
  return bytes;
}

// GimbalRateIntent { pitch_rate=1, yaw_rate=2 }
function encodeGimbalRateIntent({ pitchRate, yawRate }) {
  const bytes = [];
  fieldFloat(bytes, 1, pitchRate);
  fieldFloat(bytes, 2, yawRate);
  return bytes;
}

function encodeControlIntent({ velocity, gimbalRate }) {
  const bytes = [];
  if (velocity) fieldMessage(bytes, 1, encodeVelocityIntent(velocity));
  if (gimbalRate) fieldMessage(bytes, 5, encodeGimbalRateIntent(gimbalRate));
  return bytes;
}

// ControlActionRequest { action=1, mode_target=2, action_id=3 }. The id is
// the reliable-delivery correlation: the sender repeats the action on
// successive frames until a ControlActionResult echoes the id, and the host
// deduplicates repeats. Zero (omitted) means "no correlation".
function encodeControlActionRequest({ action, modeTarget, actionId }) {
  const bytes = [];
  fieldVarint(bytes, 1, action);
  if (modeTarget) fieldVarint(bytes, 2, modeTarget);
  if (actionId) fieldVarint(bytes, 3, actionId);
  return bytes;
}

/**
 * Encodes one `ControlFrame` wrapped in an `Envelope`, ready to send as a
 * single control-fast datagram (bare envelope, no length prefix, ADR-0005).
 *
 * A frame carries EXACTLY ONE command representation: the typed
 * `velocity`/`gimbalRate` intent (physical units inside the advertised
 * envelope) plus typed `actions`, OR the legacy `axes`/`edges` payload —
 * the host rejects both-or-neither. Production frames are typed; the
 * legacy parameters remain for wire round-trip tests of the host's
 * compatibility boundary.
 */
export function encodeControlFrameEnvelope({
  sessionId,
  vehicleId,
  scope,
  generation,
  sequence,
  sampledAtNanos,
  profileRevision,
  activationRevision,
  velocity,
  gimbalRate,
  actions,
  axes,
  edges,
}) {
  // These seven are required message-typed fields the host's wire->domain
  // conversion demands present, so emit each even when its inner scalar is 0
  // (e.g. session id 0) — matching the Rust encoder, which never omits them.
  const frame = [];
  fieldMessage(frame, 1, encodeSessionId(sessionId));
  fieldMessage(frame, 2, encodeVehicleId(vehicleId));
  fieldMessage(frame, 3, encodeScopeId(scope));
  fieldMessage(frame, 4, encodeGeneration(generation));
  fieldMessage(frame, 5, encodeSequenceNum(sequence));
  fieldMessage(frame, 6, encodeMonoTimestamp(sampledAtNanos));
  fieldVarint(frame, 7, profileRevision);
  if (axes?.length || edges?.length) {
    fieldMessage(frame, 8, encodeControlPayload(axes ?? [], edges ?? []));
  }
  if (velocity || gimbalRate) {
    fieldMessage(frame, 9, encodeControlIntent({ velocity, gimbalRate }));
  }
  for (const action of actions ?? []) {
    fieldMessage(frame, 10, encodeControlActionRequest(action));
  }
  if (activationRevision) fieldVarint(frame, 11, activationRevision);
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.controlFrame, frame));
}

/**
 * Encodes a `ProfileActivation` announcement (reliable session stream):
 * binds the activation revision this client's frames carry to the profile
 * identity, document revision, and SHA-256 content digest (INPUT-01).
 */
export function encodeProfileActivationEnvelope({
  sessionId,
  profileId,
  profileRevision,
  activationRevision,
  digest,
  deviceProfileId,
  deviceProfileRevision,
  deviceDigest,
}) {
  const bytes = [];
  fieldMessage(bytes, 1, encodeSessionId(sessionId));
  fieldString(bytes, 2, profileId ?? "");
  fieldVarint(bytes, 3, profileRevision);
  fieldVarint(bytes, 4, activationRevision);
  fieldBytes(bytes, 5, Array.from(digest ?? []));
  // The composite mapping is scheme + device: a device selection changes
  // what physical input means, so the announcement names both documents.
  if (deviceProfileId) {
    fieldString(bytes, 6, deviceProfileId);
    fieldVarint(bytes, 7, deviceProfileRevision ?? 0);
    fieldBytes(bytes, 8, Array.from(deviceDigest ?? []));
  }
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.profileActivation, bytes));
}

function encodeSessionId(value) {
  const bytes = [];
  fieldVarint(bytes, 1, value);
  return bytes;
}

function encodeGeneration(value) {
  const bytes = [];
  fieldVarint(bytes, 1, value);
  return bytes;
}

function encodeSequenceNum(value) {
  const bytes = [];
  fieldVarint(bytes, 1, value);
  return bytes;
}

// message MonoTimestamp { uint64 nanos = 1; }
function encodeMonoTimestamp(nanos) {
  const bytes = [];
  fieldVarint(bytes, 1, nanos);
  return bytes;
}

// ---- generic decode: enough to read ServerWelcome / LeaseResponse fields --
// This is a tiny generic protobuf reader (any field, any wire type), used to
// pull out just the handful of scalar fields this viewer displays, without a
// full descriptor-driven decoder.

/** Parses a top-level protobuf message into a Map<fieldNumber, list-of-values>.
 * Each value is either a bigint (varint) or a Uint8Array (length-delimited or
 * fixed-width). Callers explicitly narrow protocol enums and u32 values.
 */
function parseFields(bytes) {
  const fields = new Map();
  let offset = 0;
  while (offset < bytes.length) {
    const [tag, afterTag] = readVarint(bytes, offset);
    const fieldNumber = tag >>> 3;
    const wireType = tag & 0x7;
    offset = afterTag;
    let value;
    if (wireType === WIRE_VARINT) {
      const [v, next] = readVarintBigInt(bytes, offset);
      value = v;
      offset = next;
    } else if (wireType === WIRE_LEN) {
      const [len, next] = readVarint(bytes, offset);
      value = bytes.subarray(next, next + len);
      offset = next + len;
    } else if (wireType === WIRE_64BIT) {
      value = bytes.subarray(offset, offset + 8);
      offset += 8;
    } else if (wireType === WIRE_32BIT) {
      value = bytes.subarray(offset, offset + 4);
      offset += 4;
    } else {
      throw new Error(`unsupported wire type ${wireType} decoding envelope`);
    }
    if (!fields.has(fieldNumber)) fields.set(fieldNumber, []);
    fields.get(fieldNumber).push(value);
  }
  return fields;
}

function firstBytes(fields, fieldNumber) {
  const values = fields.get(fieldNumber);
  return values && values.length > 0 ? values[0] : undefined;
}

function firstVarint(fields, fieldNumber) {
  const values = fields.get(fieldNumber);
  return values && values.length > 0 ? Number(values[0]) : 0;
}

function firstBigVarint(fields, fieldNumber) {
  const values = fields.get(fieldNumber);
  return values && values.length > 0 ? values[0] : 0n;
}

function decodeUint64Message(bytes) {
  if (!bytes) return 0n;
  const fields = parseFields(bytes);
  return firstBigVarint(fields, 1);
}

function decodeStringMessage(bytes) {
  if (!bytes) return "";
  const fields = parseFields(bytes);
  const raw = firstBytes(fields, 1);
  return raw ? new TextDecoder().decode(raw) : "";
}

function decodeFloat32(view) {
  if (!view || view.length < 4) return 0;
  return new DataView(view.buffer, view.byteOffset, 4).getFloat32(0, true);
}

/**
 * Decodes exactly one length-delimited `Envelope` frame from the front of
 * `bytes` (varint byte-length prefix + that many bytes), returning
 * `{ kind, message, consumed }` where `kind` is one of `"ServerWelcome"`,
 * `"LeaseResponse"`, `"AuthorityEvent"`, `"TelemetrySample"`, `"Pong"`,
 * `"FrameRejected"`, or `"unknown"`, and `consumed` is the total byte count
 * (prefix + payload) so the caller can advance its buffer.
 *
 * Returns `null` if the prefix or payload has not fully arrived yet.
 */
export function decodeLengthDelimitedEnvelope(bytes) {
  if (bytes.length === 0) return null;
  let len, afterLen;
  try {
    [len, afterLen] = readVarint(bytes, 0);
  } catch {
    return null;
  }
  if (afterLen + len > bytes.length) return null;
  const body = bytes.subarray(afterLen, afterLen + len);
  const consumed = afterLen + len;
  return { ...decodeEnvelopeBody(body), consumed };
}

/** Decodes a bare (non-length-delimited) `Envelope`, e.g. a telemetry datagram. */
export function decodeBareEnvelope(bytes) {
  return decodeEnvelopeBody(bytes);
}

/**
 * Decodes a bare `ControlFrame`, mirroring `encodeControlFrameEnvelope`'s field
 * layout. Used by the vehicle.gimbal wire round-trip test (and available to any
 * loopback consumer) to prove a gimbal control frame survives the wire.
 */
export function decodeControlFrame(bytes) {
  const f = parseFields(bytes);
  const axes = [];
  const edges = [];
  const payloadBytes = firstBytes(f, 8);
  if (payloadBytes) {
    const payload = parseFields(payloadBytes);
    for (const axisBytes of payload.get(1) ?? []) {
      const a = parseFields(axisBytes);
      axes.push([firstVarint(a, 1), decodeFloat32(firstBytes(a, 2))]);
    }
    for (const edgeBytes of payload.get(2) ?? []) {
      const e = parseFields(edgeBytes);
      edges.push([firstVarint(e, 1), firstVarint(e, 2)]);
    }
  }
  let velocity = null;
  let gimbalRate = null;
  const intentBytes = firstBytes(f, 9);
  if (intentBytes) {
    const intent = parseFields(intentBytes);
    const velocityBytes = firstBytes(intent, 1);
    if (velocityBytes) {
      const v = parseFields(velocityBytes);
      velocity = {
        frame: firstVarint(v, 1),
        vx: decodeFloat32(firstBytes(v, 2)) ?? 0,
        vy: decodeFloat32(firstBytes(v, 3)) ?? 0,
        vz: decodeFloat32(firstBytes(v, 4)) ?? 0,
        yawRate: decodeFloat32(firstBytes(v, 5)) ?? 0,
      };
    }
    const gimbalBytes = firstBytes(intent, 5);
    if (gimbalBytes) {
      const g = parseFields(gimbalBytes);
      gimbalRate = {
        pitchRate: decodeFloat32(firstBytes(g, 1)) ?? 0,
        yawRate: decodeFloat32(firstBytes(g, 2)) ?? 0,
      };
    }
  }
  const actions = (f.get(10) ?? []).map((actionBytes) => {
    const a = parseFields(actionBytes);
    return { action: firstVarint(a, 1), modeTarget: firstVarint(a, 2), actionId: firstVarint(a, 3) };
  });
  return {
    scope: decodeStringMessage(firstBytes(f, 3)),
    profileRevision: firstVarint(f, 7),
    activationRevision: firstVarint(f, 11),
    axes,
    edges,
    velocity,
    gimbalRate,
    actions,
  };
}

function decodeEnvelopeBody(body) {
  const fields = parseFields(body);
  if (fields.has(ENVELOPE_FIELD.serverWelcome)) {
    return { kind: "ServerWelcome", message: decodeServerWelcome(firstBytes(fields, ENVELOPE_FIELD.serverWelcome)) };
  }
  if (fields.has(ENVELOPE_FIELD.leaseResponse)) {
    return { kind: "LeaseResponse", message: decodeLeaseResponse(firstBytes(fields, ENVELOPE_FIELD.leaseResponse)) };
  }
  if (fields.has(ENVELOPE_FIELD.authorityEvent)) {
    return { kind: "AuthorityEvent", message: decodeAuthorityEvent(firstBytes(fields, ENVELOPE_FIELD.authorityEvent)) };
  }
  if (fields.has(ENVELOPE_FIELD.telemetrySample)) {
    return { kind: "TelemetrySample", message: decodeTelemetrySample(firstBytes(fields, ENVELOPE_FIELD.telemetrySample)) };
  }
  if (fields.has(ENVELOPE_FIELD.controlFrame)) {
    return { kind: "ControlFrame", message: decodeControlFrame(firstBytes(fields, ENVELOPE_FIELD.controlFrame)) };
  }
  if (fields.has(ENVELOPE_FIELD.pong)) {
    return { kind: "Pong", message: {} };
  }
  if (fields.has(ENVELOPE_FIELD.frameRejected)) {
    return { kind: "FrameRejected", message: decodeFrameRejected(firstBytes(fields, ENVELOPE_FIELD.frameRejected)) };
  }
  if (fields.has(ENVELOPE_FIELD.leaseReleased)) {
    return { kind: "LeaseReleased", message: decodeLeaseReleased(firstBytes(fields, ENVELOPE_FIELD.leaseReleased)) };
  }
  if (fields.has(ENVELOPE_FIELD.linkLossCleared)) {
    return { kind: "LinkLossCleared", message: decodeLinkLossCleared(firstBytes(fields, ENVELOPE_FIELD.linkLossCleared)) };
  }
  if (fields.has(ENVELOPE_FIELD.controlActionResult)) {
    return {
      kind: "ControlActionResult",
      message: decodeControlActionResult(firstBytes(fields, ENVELOPE_FIELD.controlActionResult)),
    };
  }
  return { kind: "unknown", message: {} };
}

// session.proto ServerWelcome: session=1, principal=2, host_capabilities=3, scope_holders=4
function decodeServerWelcome(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    sessionId: decodeUint64Message(firstBytes(fields, 1)),
    principalId: decodeUint64Message(firstBytes(fields, 2)),
    // The typed capability negotiation (CTRL-01): every advertised scope
    // with its intent families, limits, and actions, so the control path
    // scales by the vehicle's REAL envelope and fails closed without one.
    advertisedScopes: decodeAdvertisedScopes(firstBytes(fields, 3)),
  };
}

// session.proto LeaseResponse: vehicle=1, scope=2, granted=3, generation=4, reason=5
function decodeLeaseResponse(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    vehicleId: decodeUint64Message(firstBytes(fields, 1)),
    scope: decodeStringMessage(firstBytes(fields, 2)),
    granted: !!firstVarint(fields, 3),
    generation: decodeUint64Message(firstBytes(fields, 4)),
    reason: firstVarint(fields, 5),
  };
}

// session.proto LeaseReleased: vehicle=1, scope=2, released=3, generation=4
function decodeLeaseReleased(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    vehicleId: decodeUint64Message(firstBytes(fields, 1)),
    scope: decodeStringMessage(firstBytes(fields, 2)),
    released: !!firstVarint(fields, 3),
    generation: decodeUint64Message(firstBytes(fields, 4)),
  };
}

// session.proto LinkLossCleared: vehicle=1, scope=2, generation=3
function decodeLinkLossCleared(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    vehicleId: decodeUint64Message(firstBytes(fields, 1)),
    scope: decodeStringMessage(firstBytes(fields, 2)),
    generation: decodeUint64Message(firstBytes(fields, 3)),
  };
}

// session.proto ControlActionResult: vehicle=1, scope=2, generation=3,
// sequence=4, action=5, mode_target=6, accepted=7, detail=8, action_id=9
function decodeControlActionResult(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    vehicleId: decodeUint64Message(firstBytes(fields, 1)),
    scope: decodeStringMessage(firstBytes(fields, 2)),
    generation: decodeUint64Message(firstBytes(fields, 3)),
    sequence: decodeUint64Message(firstBytes(fields, 4)),
    action: firstVarint(fields, 5),
    modeTarget: firstVarint(fields, 6),
    accepted: !!firstVarint(fields, 7),
    detail: firstBytes(fields, 8) ? new TextDecoder().decode(firstBytes(fields, 8)) : "",
    actionId: firstVarint(fields, 9) ?? 0,
  };
}

/** Every value of a repeated varint field, handling BOTH encodings proto3
 * permits: PACKED (one length-delimited blob of varints — prost's default
 * for repeated enums) and unpacked (one varint entry per element). A naive
 * `Number(blob)` turns a multi-value packed blob into NaN. */
function repeatedVarints(fields, fieldNumber) {
  const values = [];
  for (const entry of fields.get(fieldNumber) ?? []) {
    if (entry instanceof Uint8Array) {
      let offset = 0;
      while (offset < entry.length) {
        const [value, next] = readVarint(entry, offset);
        values.push(Number(value));
        offset = next;
      }
    } else {
      values.push(Number(entry));
    }
  }
  return values;
}

// capability.proto IntentCapability: family=1, frames=2 (repeated varint),
// max_linear=3, max_angular=4, max_vertical=5
function decodeIntentCapability(bytes) {
  const fields = parseFields(bytes);
  return {
    family: firstVarint(fields, 1),
    frames: repeatedVarints(fields, 2),
    maxLinear: decodeFloat32(firstBytes(fields, 3)) ?? 0,
    maxAngular: decodeFloat32(firstBytes(fields, 4)) ?? 0,
    maxVertical: decodeFloat32(firstBytes(fields, 5)) ?? 0,
  };
}

// capability.proto ActionCapability: action=1, mode_targets=2 (repeated varint)
function decodeActionCapability(bytes) {
  const fields = parseFields(bytes);
  return {
    action: firstVarint(fields, 1),
    modeTargets: repeatedVarints(fields, 2),
  };
}

// capability.proto ScopeDescriptor: scope=1, display_name=2,
// link_loss_action=3, intents=4, actions=5
function decodeScopeDescriptor(bytes) {
  const fields = parseFields(bytes);
  return {
    scope: decodeStringMessage(firstBytes(fields, 1)),
    intents: (fields.get(4) ?? []).map(decodeIntentCapability),
    actions: (fields.get(5) ?? []).map(decodeActionCapability),
  };
}

// capability.proto: HostCapabilities.vehicles=2; VehicleDescriptor.vehicle=1,
// scopes=3. Extracts the typed capability negotiation the control path
// scales by (CTRL-01) — one flat list of per-vehicle scope descriptors.
function decodeAdvertisedScopes(hostCapabilitiesBytes) {
  if (!hostCapabilitiesBytes) return [];
  const caps = parseFields(hostCapabilitiesBytes);
  const scopes = [];
  for (const vehicleBytes of caps.get(2) ?? []) {
    const vehicle = parseFields(vehicleBytes);
    const vehicleId = decodeUint64Message(firstBytes(vehicle, 1));
    for (const scopeBytes of vehicle.get(3) ?? []) {
      scopes.push({ vehicleId, ...decodeScopeDescriptor(scopeBytes) });
    }
  }
  return scopes;
}

// telemetry.proto TelemetrySample: vehicle=1, tick=2, observed_at=3, pose=4,
// velocity=5, avionics=6, sim_truth=7, fc_state=8, gimbal=9
// Pose2d: x_m=1, y_m=2, heading_rad=3 (all float, wire type 5)
function decodeTelemetrySample(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  const pose = decodePose2d(firstBytes(fields, 4));
  const velocity = decodeVelocity2d(firstBytes(fields, 5));
  return {
    vehicleId: decodeUint64Message(firstBytes(fields, 1)),
    tick: decodeUint64Message(firstBytes(fields, 2)),
    publishedAtNanos: decodeUint64Message(firstBytes(fields, 3)),
    pose,
    velocity,
    xM: pose?.xM ?? null,
    yM: pose?.yM ?? null,
    headingRad: pose?.headingRad ?? null,
    linearXMps: velocity?.linearXMps ?? null,
    angularRadS: velocity?.angularRadS ?? null,
    avionics: decodeAvionicsState(firstBytes(fields, 6)),
    simTruth: decodeSimTruthState(firstBytes(fields, 7)),
    fcState: decodeFcState(firstBytes(fields, 8)),
    gimbal: decodeGimbalAttitude(firstBytes(fields, 9)),
  };
}

// telemetry.proto SimTruthState: quat_w..z=1..4, pos_n/e/d_m=5..7,
// vel_n/e/d_mps=8..10 (float), stamp=11, valid_flags=12, integrity=13.
// The simulation-truth oracle: structurally separate from the avionics
// estimate, and unconsumable (null) without its provenance stamp.
function decodeSimTruthState(bytes) {
  if (!bytes) return null;
  const f = parseFields(bytes);
  const stamp = decodeMeasurementStamp(firstBytes(f, 11));
  // Exact-role gate: a truth lane whose stamp does not carry the
  // simulation-truth role is mislabeled and unconsumable.
  if (stamp === null || stamp.role !== 2) return null;
  return {
    quat: {
      w: decodeFloat32(firstBytes(f, 1)),
      x: decodeFloat32(firstBytes(f, 2)),
      y: decodeFloat32(firstBytes(f, 3)),
      z: decodeFloat32(firstBytes(f, 4)),
    },
    posNed: [
      decodeFloat32(firstBytes(f, 5)),
      decodeFloat32(firstBytes(f, 6)),
      decodeFloat32(firstBytes(f, 7)),
    ],
    velNed: [
      decodeFloat32(firstBytes(f, 8)),
      decodeFloat32(firstBytes(f, 9)),
      decodeFloat32(firstBytes(f, 10)),
    ],
    validFlags: firstVarint(f, 12) ?? 0,
    stamp,
  };
}

// telemetry.proto FcState: arm_state=1 (varint), stamp=2. FC-owned arm
// state under its own provenance; unconsumable (null) without the stamp.
function decodeFcState(bytes) {
  if (!bytes) return null;
  const f = parseFields(bytes);
  const stamp = decodeMeasurementStamp(firstBytes(f, 2));
  // Exact-role gate: FC state must carry the FC-state role.
  if (stamp === null || stamp.role !== 3) return null;
  return {
    armState: firstVarint(f, 1) ?? 0,
    stamp,
  };
}

// telemetry.proto GimbalAttitude: quat_w..z=1..4, rate_x/y/z_rad_s=5..7,
// stamp=8, flags=9, failure_flags=10. Payload-device orientation under its
// own provenance; unconsumable (null) without a payload-device stamp, so a
// mislabeled lane can never point the camera view or be read as FC state.
function decodeGimbalAttitude(bytes) {
  if (!bytes) return null;
  const f = parseFields(bytes);
  const stamp = decodeMeasurementStamp(firstBytes(f, 8));
  // Exact-role gate: the gimbal lane must carry the payload-device role.
  if (stamp === null || stamp.role !== 5) return null;
  return {
    quat: {
      w: decodeFloat32(firstBytes(f, 1)),
      x: decodeFloat32(firstBytes(f, 2)),
      y: decodeFloat32(firstBytes(f, 3)),
      z: decodeFloat32(firstBytes(f, 4)),
    },
    ratesRadS: [
      decodeFloat32(firstBytes(f, 5)),
      decodeFloat32(firstBytes(f, 6)),
      decodeFloat32(firstBytes(f, 7)),
    ],
    flags: firstVarint(f, 9) ?? 0,
    failureFlags: firstVarint(f, 10) ?? 0,
    stamp,
  };
}

function decodePose2d(bytes) {
  if (!bytes) return null;
  const fields = parseFields(bytes);
  return {
    xM: decodeFloat32(firstBytes(fields, 1)),
    yM: decodeFloat32(firstBytes(fields, 2)),
    headingRad: decodeFloat32(firstBytes(fields, 3)),
  };
}

function decodeVelocity2d(bytes) {
  if (!bytes) return null;
  const fields = parseFields(bytes);
  return {
    linearXMps: decodeFloat32(firstBytes(fields, 1)),
    linearYMps: decodeFloat32(firstBytes(fields, 2)),
    angularRadS: decodeFloat32(firstBytes(fields, 3)),
  };
}

// telemetry.proto AvionicsState (ADR-0018): quat_w..z=1..4,
// rate_p/q/r_rad_s=5..7, pos_n/e/d_m=8..10, vel_n/e/d_mps=11..13 (float),
// valid_flags=14, quality=15, arm_state=16 (varint), attitude and kinematics
// stamps=17/18, estimator authorization stamp=19.
// Raw estimate; display derivation
// happens in the instrument runtime (ADR-0017).
function decodeAvionicsState(bytes) {
  if (!bytes) return null;
  const f = parseFields(bytes);
  const attitudeStamp = decodeMeasurementStamp(firstBytes(f, 17));
  const kinematicsStamp = decodeMeasurementStamp(firstBytes(f, 18));
  const estimatorStatusStamp = decodeMeasurementStamp(firstBytes(f, 19));
  const attitude = attitudeStamp === null ? null : {
    quat: {
      w: decodeFloat32(firstBytes(f, 1)),
      x: decodeFloat32(firstBytes(f, 2)),
      y: decodeFloat32(firstBytes(f, 3)),
      z: decodeFloat32(firstBytes(f, 4)),
    },
    rates: [
      decodeFloat32(firstBytes(f, 5)),
      decodeFloat32(firstBytes(f, 6)),
      decodeFloat32(firstBytes(f, 7)),
    ],
  };
  const kinematics = kinematicsStamp === null ? null : {
    posNed: [
      decodeFloat32(firstBytes(f, 8)),
      decodeFloat32(firstBytes(f, 9)),
      decodeFloat32(firstBytes(f, 10)),
    ],
    velNed: [
      decodeFloat32(firstBytes(f, 11)),
      decodeFloat32(firstBytes(f, 12)),
      decodeFloat32(firstBytes(f, 13)),
    ],
  };
  return {
    attitude,
    kinematics,
    quat: attitude?.quat ?? null,
    rates: attitude?.rates ?? null,
    posNed: kinematics?.posNed ?? null,
    velNed: kinematics?.velNed ?? null,
    validFlags: firstVarint(f, 14) ?? 0,
    quality: firstVarint(f, 15) ?? 0,
    // 0 unknown, 1 disarmed, 2 armed.
    armState: firstVarint(f, 16) ?? 0,
    attitudeStamp,
    kinematicsStamp,
    estimatorStatusStamp,
  };
}

function decodeMeasurementStamp(bytes) {
  if (!bytes) return null;
  const f = parseFields(bytes);
  return {
    sourceId: firstBigVarint(f, 1),
    sourceIncarnation: decodeIncarnation(firstBytes(f, 6)),
    // No `>>> 0`: a wire value past u32 must be REJECTED by the identity
    // validator downstream, never silently truncated into range.
    sourceEpoch: firstVarint(f, 2),
    sequence: firstVarint(f, 3),
    acquiredAtNanos: firstBigVarint(f, 4),
    clock: firstVarint(f, 5),
    // Explicit source role (1 estimate, 2 simulation truth, 3 FC state,
    // 4 video capture); consumers gate on this, never on id ranges.
    role: firstVarint(f, 7) ?? 0,
    // Integrity of the delivering path (1 authenticated, 2 checksummed
    // only, 3 unprotected; 0 unspecified).
    integrity: firstVarint(f, 8) ?? 0,
  };
}

function decodeIncarnation(bytes) {
  if (!bytes || bytes.length !== 16) return null;
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

// authority.proto AuthorityEvent: a oneof; we only care which arm fired and a
// human-readable label, not full field decode, for the demo overlay.
const AUTHORITY_ARM_NAMES = {
  1: "ScopeLeaseGranted",
  2: "ScopeTransferOffered",
  3: "ScopeTransferAccepted",
  4: "ScopeTransferCommitted",
  5: "ScopeLeaseRevoked",
  6: "EmergencyOverrideApplied",
  7: "ScopeRegistered",
  8: "ScopeTransferExpired",
  9: "LinkStateChanged",
  10: "WarningRaised",
};

function decodeAuthorityEvent(bytes) {
  if (!bytes) return { arm: "unknown" };
  const fields = parseFields(bytes);
  for (const fieldNumber of fields.keys()) {
    if (AUTHORITY_ARM_NAMES[fieldNumber]) {
      return { arm: AUTHORITY_ARM_NAMES[fieldNumber] };
    }
  }
  return { arm: "unknown" };
}

// session.proto FrameRejected: vehicle=1, scope=2, sequence=3, reason=4, current_generation=5
function decodeFrameRejected(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return { reason: firstVarint(fields, 4) };
}
