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

import { encodeControlFrameEnvelope, decodeBareEnvelope, SCHEMA_VERSION } from "./wire.js";
import "./telemetry-ingress.test.mjs";

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
  let n = v;
  for (;;) {
    const b = n & 0x7f;
    n >>>= 7;
    if (n === 0) {
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

function measurementStamp(sourceId, epoch, sequence, acquiredAtNanos, clock) {
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
  return out;
}

const avionics = [];
f32Field(avionics, 1, 0.9); // quat_w; x/y/z stay 0 and are omitted
f32Field(avionics, 7, 0.05); // rate_r
f32Field(avionics, 10, -304.8); // pos_d
f32Field(avionics, 11, 10.0); // vel_n
varint(avionics, (14 << 3) | 0);
varint(avionics, 0b1111); // valid_flags
bytesField(avionics, 17, measurementStamp(7, 3, 10, 1_000_000, 1));
bytesField(avionics, 18, measurementStamp(7, 3, 5, 900_000, 1));

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
check("avionics quat_w decodes", Math.abs(av.quat.w - 0.9) < 1e-6);
check("omitted zero quat components decode as 0", av.quat.x === 0 && av.quat.y === 0 && av.quat.z === 0);
check("avionics pos_d decodes", Math.abs(av.posNed[2] + 304.8) < 1e-3);
check("avionics vel_n decodes", Math.abs(av.velNed[0] - 10.0) < 1e-6);
check("avionics valid_flags decode", Number(av.validFlags) === 0b1111);
check("attitude measurement identity decodes", av.attitudeStamp.sourceId === 7n);
check("attitude epoch and sequence decode", av.attitudeStamp.sourceEpoch === 3 && av.attitudeStamp.sequence === 10);
check("attitude acquisition time and clock decode", av.attitudeStamp.acquiredAtNanos === 1_000_000n && av.attitudeStamp.clock === 1);
check("kinematics sequence remains independent", av.kinematicsStamp.sequence === 5);
check("a sample without avionics decodes to null", (() => {
  const bareNoAv = [];
  varint(bareNoAv, (1 << 3) | 0);
  varint(bareNoAv, SCHEMA_VERSION);
  bytesField(bareNoAv, 4, []);
  return decodeBareEnvelope(new Uint8Array(bareNoAv)).message.avionics === null;
})());

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall wire conformance checks passed");
