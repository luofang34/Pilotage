// Conformance checks for the hand-rolled protobuf encoder (wire.js).
//
// Run: node clients/web/wire.test.mjs
//
// The host's wire->domain ControlFrame conversion requires every message-typed
// field present (session, vehicle, scope, generation, sequence, sampled_at,
// payload). proto3 omits scalar defaults, so a naive encoder drops a
// message-typed field whose inner id is 0 (e.g. session 0), and the host then
// rejects the datagram as unrecognized. These checks pin the invariant that the
// required fields are emitted even when their ids are zero.

import {
  encodeControlFrameEnvelope,
  decodeBareEnvelope,
  parseVideoFrameV2,
  SCHEMA_VERSION,
} from "./wire.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// Minimal protobuf field walker: returns a Map of field number -> the raw bytes
// (length-delimited fields) or numeric value (varint), enough to assert field
// presence and descend into one submessage.
function walk(bytes) {
  const out = new Map();
  let i = 0;
  while (i < bytes.length) {
    let tag = 0;
    let shift = 0;
    for (;;) {
      const b = bytes[i++];
      tag |= (b & 0x7f) << shift;
      if ((b & 0x80) === 0) break;
      shift += 7;
    }
    const fieldNumber = tag >>> 3;
    const wireType = tag & 0x7;
    if (wireType === 0) {
      let v = 0n;
      let s = 0n;
      for (;;) {
        const b = bytes[i++];
        v |= BigInt(b & 0x7f) << s;
        if ((b & 0x80) === 0) break;
        s += 7n;
      }
      out.set(fieldNumber, v);
    } else if (wireType === 5) {
      out.set(fieldNumber, bytes.subarray(i, i + 4));
      i += 4;
    } else if (wireType === 2) {
      let len = 0;
      let s = 0;
      for (;;) {
        const b = bytes[i++];
        len |= (b & 0x7f) << s;
        if ((b & 0x80) === 0) break;
        s += 7;
      }
      out.set(fieldNumber, bytes.subarray(i, i + len));
      i += len;
    } else {
      throw new Error(`unexpected wire type ${wireType}`);
    }
  }
  return out;
}

// The dangerous case: session id, vehicle id, generation, sequence all 0.
const envelope = encodeControlFrameEnvelope({
  sessionId: 0,
  vehicleId: 0n,
  scope: "vehicle.motion",
  generation: 0n,
  sequence: 0,
  sampledAtNanos: 0n,
  profileRevision: 1,
  axes: [
    [2, 0.0],
    [3, 0.0],
  ],
});

const top = walk(envelope);
check("envelope carries the supported schema version", top.get(1) === BigInt(SCHEMA_VERSION));
check("envelope carries the control_frame arm (field 2)", top.has(2));

const frame = walk(top.get(2));
for (const [num, name] of [
  [1, "session"],
  [2, "vehicle"],
  [3, "scope"],
  [4, "generation"],
  [5, "sequence"],
  [6, "sampled_at"],
  [8, "payload"],
]) {
  check(`control frame emits required field ${num} (${name}) even when zero`, frame.has(num));
}

// ---- telemetry avionics decode (ADR-0018) -----------------------------------
// Builds a bare Envelope { schema_version, telemetry_sample { avionics } }
// byte-by-byte and asserts the decoder surfaces the raw estimate, including
// proto3's omitted zero-valued fields (quat_x/y/z absent -> 0).

function varint(out, v) {
  let n = BigInt(v);
  for (;;) {
    const b = Number(n & 0x7fn);
    n >>= 7n;
    if (n === 0n) {
      out.push(b);
      return;
    }
    out.push(b | 0x80);
  }
}
function f32Field(out, fieldNumber, value) {
  if (value === 0) return; // proto3 omits defaults
  varint(out, (fieldNumber << 3) | 5);
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, value, true);
  out.push(...b);
}
function bytesField(out, fieldNumber, bytes) {
  varint(out, (fieldNumber << 3) | 2);
  varint(out, bytes.length);
  out.push(...bytes);
}

function uint64Message(value) {
  const out = [];
  varint(out, (1 << 3) | 0);
  varint(out, value);
  return out;
}

function measurementStamp(sourceId, epoch, sequence, acquiredAtNanos, clock, incarnationByte) {
  const out = [];
  for (const [field, value] of [
    [1, sourceId],
    [2, epoch],
    [3, sequence],
    [4, acquiredAtNanos],
    [5, clock],
  ]) {
    varint(out, (field << 3) | 0);
    varint(out, value);
  }
  bytesField(out, 6, new Uint8Array(16).fill(incarnationByte));
  return out;
}

function decodeAvionicsOnly(avionicsBytes) {
  const telemetry = [];
  bytesField(telemetry, 1, uint64Message(1));
  bytesField(telemetry, 2, uint64Message(42));
  bytesField(telemetry, 6, avionicsBytes);
  const envelopeBytes = [];
  varint(envelopeBytes, (1 << 3) | 0);
  varint(envelopeBytes, SCHEMA_VERSION);
  bytesField(envelopeBytes, 4, telemetry);
  return decodeBareEnvelope(new Uint8Array(envelopeBytes)).message;
}

const avionics = [];
f32Field(avionics, 1, 0.9); // quat_w; x/y/z stay 0 and are omitted
f32Field(avionics, 7, 0.05); // rate_r
f32Field(avionics, 10, -304.8); // pos_d
f32Field(avionics, 11, 10.0); // vel_n
varint(avionics, (14 << 3) | 0);
varint(avionics, 0b1111); // valid_flags
const sourceId = 0xfedc_ba98_7654_3210n;
const acquiredAt = 0xffff_ffff_ffff_fffen;
bytesField(avionics, 17, measurementStamp(sourceId, 3, 10, acquiredAt, 1, 0xa5));
bytesField(avionics, 18, measurementStamp(sourceId, 3, 5, acquiredAt - 100_000n, 1, 0xa5));
bytesField(avionics, 19, measurementStamp(sourceId, 3, 12, acquiredAt, 1, 0xa5));

const sample = [];
bytesField(sample, 1, uint64Message(1));
bytesField(sample, 2, uint64Message(42));
bytesField(sample, 3, uint64Message(2_000_000));
bytesField(sample, 6, avionics);
const bare = [];
varint(bare, (1 << 3) | 0);
varint(bare, SCHEMA_VERSION);
bytesField(bare, 4, sample);

const decoded = decodeBareEnvelope(new Uint8Array(bare));
check("telemetry datagram decodes as TelemetrySample", decoded.kind === "TelemetrySample");
const av = decoded.message.avionics;
check("avionics arm is surfaced", !!av);
check("telemetry vehicle identity is preserved", decoded.message.vehicleId === 1n);
check("telemetry source tick is preserved", decoded.message.tick === 42n);
check("host publication time is preserved separately", decoded.message.publishedAtNanos === 2_000_000n);
check("absent top-level pose remains null", decoded.message.pose === null && decoded.message.xM === null);
check("absent top-level velocity remains null", decoded.message.velocity === null && decoded.message.linearXMps === null);
check("both stamped numeric groups are present", av.attitude !== null && av.kinematics !== null);
check("avionics quat_w decodes", Math.abs(av.quat.w - 0.9) < 1e-6);
check("omitted zero quat components decode as 0", av.quat.x === 0 && av.quat.y === 0 && av.quat.z === 0);
check("avionics pos_d decodes", Math.abs(av.posNed[2] + 304.8) < 1e-3);
check("avionics vel_n decodes", Math.abs(av.velNed[0] - 10.0) < 1e-6);
check("avionics valid_flags decode", Number(av.validFlags) === 0b1111);
check("attitude uint64 identity decodes without precision loss", av.attitudeStamp.sourceId === sourceId);
check("attitude incarnation decodes exactly", av.attitudeStamp.sourceIncarnation === "a5".repeat(16));
check("attitude epoch and sequence decode", av.attitudeStamp.sourceEpoch === 3 && av.attitudeStamp.sequence === 10);
check("attitude uint64 acquisition time and clock decode", av.attitudeStamp.acquiredAtNanos === acquiredAt && av.attitudeStamp.clock === 1);
check("kinematics sequence remains independent", av.kinematicsStamp.sequence === 5);
check("estimator status stamp decodes independently", av.estimatorStatusStamp.sequence === 12);

const attitudeOnly = [];
f32Field(attitudeOnly, 1, 0.7);
f32Field(attitudeOnly, 7, 0.2);
f32Field(attitudeOnly, 8, 99.0); // ignored without a kinematics stamp
bytesField(attitudeOnly, 17, measurementStamp(sourceId, 3, 11, acquiredAt, 1, 0xa5));
const decodedAttitudeOnly = decodeAvionicsOnly(attitudeOnly);
check(
  "attitude-only publication omits top-level pose and velocity",
  decodedAttitudeOnly.pose === null && decodedAttitudeOnly.velocity === null,
);
check(
  "attitude-only publication preserves only the attitude group",
  decodedAttitudeOnly.avionics.attitude !== null
    && decodedAttitudeOnly.avionics.kinematics === null
    && decodedAttitudeOnly.avionics.estimatorStatusStamp === null
    && decodedAttitudeOnly.avionics.quat !== null
    && decodedAttitudeOnly.avionics.posNed === null,
);

const kinematicsOnly = [];
f32Field(kinematicsOnly, 1, 0.9); // ignored without an attitude stamp
f32Field(kinematicsOnly, 8, 12.0);
f32Field(kinematicsOnly, 11, 5.0);
bytesField(kinematicsOnly, 18, measurementStamp(sourceId, 3, 6, acquiredAt, 1, 0xa5));
const decodedKinematicsOnly = decodeAvionicsOnly(kinematicsOnly);
check(
  "kinematics-only publication omits top-level pose and velocity",
  decodedKinematicsOnly.pose === null && decodedKinematicsOnly.velocity === null,
);
check(
  "kinematics-only publication preserves only the kinematics group",
  decodedKinematicsOnly.avionics.attitude === null
    && decodedKinematicsOnly.avionics.kinematics !== null
    && decodedKinematicsOnly.avionics.estimatorStatusStamp === null
    && decodedKinematicsOnly.avionics.quat === null
    && decodedKinematicsOnly.avionics.posNed[0] === 12,
);

const statusOnly = [];
varint(statusOnly, (15 << 3) | 0);
varint(statusOnly, 2); // canonical Unusable; valid_flags remains omitted/zero
bytesField(statusOnly, 19, measurementStamp(sourceId, 3, 13, acquiredAt + 1n, 1, 0xa5));
const decodedStatusOnly = decodeAvionicsOnly(statusOnly).avionics;
check(
  "status-only publication preserves authorization without numeric groups",
  decodedStatusOnly.attitude === null
    && decodedStatusOnly.kinematics === null
    && decodedStatusOnly.validFlags === 0
    && decodedStatusOnly.quality === 2
    && decodedStatusOnly.estimatorStatusStamp.sequence === 13,
);
check("a sample without avionics decodes to null", (() => {
  const bareNoAv = [];
  varint(bareNoAv, (1 << 3) | 0);
  varint(bareNoAv, SCHEMA_VERSION);
  bytesField(bareNoAv, 4, []);
  return decodeBareEnvelope(new Uint8Array(bareNoAv)).message.avionics === null;
})());

// ---- v2 video capture-identity frame parsing (ADR-0020) --------------------
// Builds a v2 body exactly as hosts/session-host stream_tag.rs
// `frame_video_payload_v2` does, so parseVideoFrameV2 is checked against the
// real byte layout, not against itself.
function buildV2Body(fields, fourcc, payload) {
  const header = new Uint8Array(76);
  const view = new DataView(header.buffer);
  header[0] = fields.sourceId;
  view.setUint32(1, fields.sourceEpoch, true);
  header.set(fields.incarnation, 5);
  view.setUint32(21, fields.sequence, true);
  view.setBigUint64(25, fields.captureTimeNanos, true);
  header[33] = fields.captureClock;
  header[34] = fields.mappingAvailable;
  header[35] = fields.mappingTargetClock;
  view.setBigInt64(36, fields.mappingOffsetNanos, true);
  view.setBigUint64(44, fields.clockErrorBoundNanos, true);
  view.setBigUint64(52, fields.receiveTimeNanos, true);
  view.setBigUint64(60, fields.publicationTimeNanos, true);
  view.setUint32(68, fields.cameraId, true);
  view.setUint32(72, fields.calibrationId, true);
  const tail = new Uint8Array(8 + payload.length);
  const tailView = new DataView(tail.buffer);
  for (let i = 0; i < 4; i += 1) tail[i] = fourcc.charCodeAt(i);
  tailView.setUint32(4, payload.length, true);
  tail.set(payload, 8);
  const body = new Uint8Array(header.length + tail.length);
  body.set(header, 0);
  body.set(tail, header.length);
  return body;
}

const v2Fields = {
  sourceId: 1,
  sourceEpoch: 7,
  incarnation: new Uint8Array(16).fill(0xab),
  sequence: 42,
  captureTimeNanos: 123456n,
  captureClock: 2,
  mappingAvailable: 1,
  mappingTargetClock: 1,
  mappingOffsetNanos: -1000n,
  clockErrorBoundNanos: 250n,
  receiveTimeNanos: 5000n,
  publicationTimeNanos: 6000n,
  cameraId: 9,
  calibrationId: 3,
};

const v2Payload = new Uint8Array([0xff, 0xd8, 1, 2, 3, 0xff, 0xd9]);
const v2Body = buildV2Body(v2Fields, "MJPG", v2Payload);
const v2Parsed = parseVideoFrameV2(v2Body);
check("v2 frame parses to a full capture identity", (() => {
  if (!v2Parsed) return false;
  const m = v2Parsed.meta;
  return (
    m.sourceId === 1 &&
    m.sourceEpoch === 7 &&
    m.sourceIncarnation === "ab".repeat(16) &&
    m.sequence === 42 &&
    m.captureTimeNanos === 123456n &&
    m.captureClock === 2 &&
    m.mappingAvailable === true &&
    m.mappingTargetClock === 1 &&
    m.mappingOffsetNanos === -1000n &&
    m.clockErrorBoundNanos === 250n &&
    m.receiveTimeNanos === 5000n &&
    m.publicationTimeNanos === 6000n &&
    m.cameraId === 9 &&
    m.calibrationId === 3 &&
    v2Parsed.fourcc === "MJPG"
  );
})());
check(
  "v2 frame preserves the exact payload bytes",
  v2Parsed !== null &&
    v2Parsed.payload.length === v2Payload.length &&
    v2Parsed.payload.every((b, i) => b === v2Payload[i]),
);
check(
  "v2 unavailable mapping parses with a false flag",
  (() => {
    const body = buildV2Body({ ...v2Fields, mappingAvailable: 0 }, "MJPG", v2Payload);
    const parsed = parseVideoFrameV2(body);
    return parsed !== null && parsed.meta.mappingAvailable === false;
  })(),
);
check("v2 body shorter than the header is rejected", parseVideoFrameV2(new Uint8Array(80)) === null);
check(
  "v2 declared length mismatch is rejected",
  (() => {
    const body = buildV2Body(v2Fields, "MJPG", v2Payload);
    // Corrupt the u32 length prefix (offset 80) to over-declare the payload.
    new DataView(body.buffer).setUint32(80, 999, true);
    return parseVideoFrameV2(body) === null;
  })(),
);


// ---- GEO-68: the stamp decoder surfaces over-range values, never clamps -----

{
  const over = 0x1_0000_0000; // 2^32, one past u32 max
  const av = [];
  bytesField(av, 17, measurementStamp(5n, over, over, 100n, 1, 0xab));
  const decoded = decodeAvionicsOnly(av).avionics.attitudeStamp;
  check("a source_epoch past u32 is surfaced raw, not clamped to 0", decoded.sourceEpoch === over);
  check("a sequence past u32 is surfaced raw, not clamped to 0", decoded.sequence === over);
}
{
  const max = 0xffff_ffff;
  const av = [];
  bytesField(av, 17, measurementStamp(5n, max, max, 100n, 1, 0xab));
  const decoded = decodeAvionicsOnly(av).avionics.attitudeStamp;
  check("the exact u32 max round-trips through decode unchanged", decoded.sourceEpoch === max);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall wire conformance checks passed");
