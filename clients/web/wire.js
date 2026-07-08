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

// The `pilotage.v1` schema version this build produces and accepts
// (pilotage_protocol::convert::SCHEMA_VERSION mirrors this constant).
export const SCHEMA_VERSION = 1;

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

/** Reads an unsigned LEB128 varint starting at `view[offset]`; returns [value, nextOffset]. */
function readVarint(view, offset) {
  let result = 0n;
  let shift = 0n;
  let pos = offset;
  for (;;) {
    const byte = view[pos];
    pos += 1;
    result |= BigInt(byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return [Number(result), pos];
    }
    shift += 7n;
  }
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
// frame_rejected=12.
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

function encodeControlPayload(axes) {
  const bytes = [];
  for (const [axisId, value] of axes) {
    fieldBytes(bytes, 1, encodeAxisSample(axisId, value));
  }
  return bytes;
}

/**
 * Encodes one `ControlFrame` wrapped in an `Envelope`, ready to send as a
 * single control-fast datagram (bare envelope, no length prefix, ADR-0005).
 *
 * `axes` is an array of `[axisId, value]` pairs, value in [-1.0, 1.0].
 */
export function encodeControlFrameEnvelope({
  sessionId,
  vehicleId,
  scope,
  generation,
  sequence,
  sampledAtNanos,
  profileRevision,
  axes,
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
  fieldMessage(frame, 8, encodeControlPayload(axes));
  return new Uint8Array(encodeEnvelope(ENVELOPE_FIELD.controlFrame, frame));
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
 * Each value is either a number (varint), a Uint8Array (length-delimited), or
 * a number (32/64-bit fixed, returned as a BigInt-safe Number when possible).
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
      const [v, next] = readVarint(bytes, offset);
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
  return values && values.length > 0 ? values[0] : 0;
}

function decodeUint64Message(bytes) {
  if (!bytes) return 0;
  const fields = parseFields(bytes);
  return firstVarint(fields, 1);
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
  if (fields.has(ENVELOPE_FIELD.pong)) {
    return { kind: "Pong", message: {} };
  }
  if (fields.has(ENVELOPE_FIELD.frameRejected)) {
    return { kind: "FrameRejected", message: decodeFrameRejected(firstBytes(fields, ENVELOPE_FIELD.frameRejected)) };
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
  };
}

// session.proto LeaseResponse: vehicle=1, scope=2, granted=3, generation=4, reason=5
function decodeLeaseResponse(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  return {
    granted: !!firstVarint(fields, 3),
    generation: decodeUint64Message(firstBytes(fields, 4)),
    reason: firstVarint(fields, 5),
  };
}

// telemetry.proto TelemetrySample: vehicle=1, tick=2, observed_at=3, pose=4, velocity=5
// Pose2d: x_m=1, y_m=2, heading_rad=3 (all float, wire type 5)
function decodeTelemetrySample(bytes) {
  if (!bytes) return {};
  const fields = parseFields(bytes);
  const poseBytes = firstBytes(fields, 4);
  const velBytes = firstBytes(fields, 5);
  const pose = poseBytes ? parseFields(poseBytes) : new Map();
  const vel = velBytes ? parseFields(velBytes) : new Map();
  return {
    xM: decodeFloat32(firstBytes(pose, 1)),
    yM: decodeFloat32(firstBytes(pose, 2)),
    headingRad: decodeFloat32(firstBytes(pose, 3)),
    linearXMps: decodeFloat32(firstBytes(vel, 1)),
    angularRadS: decodeFloat32(firstBytes(vel, 3)),
  };
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
