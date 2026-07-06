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

import { encodeControlFrameEnvelope, SCHEMA_VERSION } from "./wire.js";

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

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall wire conformance checks passed");
