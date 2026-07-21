// The PRODUCTION long-lived uni-stream reader — `readUniStream`, the core of
// main.js's `readOneUniStream` — driven end to end against a REAL
// ReadableStream, together with the real recovery-state transition
// (`applyMotionRecovery`). This exercises what the helper-only test cannot: the
// leading 0x01 authority kind tag, a LinkLossCleared arriving FRAGMENTED on an
// OPEN (never-closing) stream, and the motionRecovered latch flipping LIVE —
// before any close — and ONLY for the matching generation.
//
// Run: node clients/web/uni-stream.test.mjs

import { readUniStream } from "./uni-stream.js";
import { applyMotionRecovery } from "./motion-lease.js";
import { decodeLengthDelimitedEnvelope, STREAM_KIND_AUTHORITY } from "./wire.js";

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

// ---- length-delimited LinkLossCleared (envelope oneof tag 15) construction ----
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
function bytesField(out, field, bytes) {
  varint(out, (field << 3) | 2);
  varint(out, bytes.length);
  out.push(...bytes);
}
function uint64Message(v) {
  const out = [];
  varint(out, (1 << 3) | 0);
  varint(out, v);
  return out;
}
function stringMessage(s) {
  const out = [];
  bytesField(out, 1, new TextEncoder().encode(s));
  return out;
}
function linkLossClearedLD(vehicle, scope, generation) {
  const cleared = [];
  bytesField(cleared, 1, uint64Message(vehicle));
  bytesField(cleared, 2, stringMessage(scope));
  bytesField(cleared, 3, uint64Message(generation));
  const envelope = [];
  bytesField(envelope, 15, cleared);
  const ld = [];
  varint(ld, envelope.length);
  ld.push(...envelope);
  return new Uint8Array(ld);
}

function taggedFirstChunk(...bytes) {
  return new Uint8Array([STREAM_KIND_AUTHORITY, ...bytes]);
}

// A hand-driven ReadableStream: `push` enqueues a chunk on the OPEN stream,
// `close` ends it. Nothing is enqueued until we say so, so we can assert what
// happened while the stream was still open — exactly the long-lived authority
// stream's shape.
function manualStream() {
  let controller;
  const stream = new ReadableStream({
    start(c) {
      controller = c;
    },
  });
  return {
    reader: stream.getReader(),
    push: (chunk) => controller.enqueue(chunk),
    close: () => controller.close(),
  };
}

// Wires readUniStream to the real recovery transition over `state`, recording
// each generation that actually confirmed recovery.
function startReader(reader, state, vehicleId, motionScope) {
  const confirmed = [];
  const done = readUniStream(reader, {
    authorityKind: STREAM_KIND_AUTHORITY,
    decode: decodeLengthDelimitedEnvelope,
    onAuthorityEnvelope: (decoded) => {
      const before = state.motionRecovered;
      applyMotionRecovery(decoded, state, vehicleId, motionScope);
      if (!before && state.motionRecovered) confirmed.push(decoded.message.generation);
    },
  });
  return { done, confirmed };
}

const VEHICLE_ID = 7n;
const MOTION_SCOPE = "vehicle.motion";

// ---- 0x01 tag + fragmented OPEN stream + LIVE recovery transition ----
await (async () => {
  const state = { generation: 42n, motionRecovered: false };
  const ack = linkLossClearedLD(7, MOTION_SCOPE, 42);
  const { reader, push, close } = manualStream();
  const { done, confirmed } = startReader(reader, state, VEHICLE_ID, MOTION_SCOPE);

  // The kind tag and only the first bytes of the ack arrive: nothing confirms
  // yet, and the stream is still open.
  push(taggedFirstChunk(...ack.subarray(0, 3)));
  await tick();
  check("a partial envelope on an open stream does not confirm", state.motionRecovered === false);

  // The remainder arrives — still on the OPEN stream (we have not closed it):
  // the envelope completes and recovery latches LIVE.
  push(ack.subarray(3));
  await tick();
  check("recovery latches LIVE from the fragmented OPEN stream", state.motionRecovered === true);
  check("it confirmed exactly once, on the acked generation", confirmed.length === 1 && confirmed[0] === 42n);

  close();
  const { kind, tail, aborted } = await done;
  check("the leading 0x01 tag was read as the authority kind", kind === STREAM_KIND_AUTHORITY);
  check("no video tail is left on the authority stream", tail.length === 0);
  check("the reader was not aborted", aborted === false);
})();

// ---- resumes ONLY the matching generation, through the real reader ----
await (async () => {
  const state = { generation: 42n, motionRecovered: false };
  const { reader, push, close } = manualStream();
  const { done, confirmed } = startReader(reader, state, VEHICLE_ID, MOTION_SCOPE);

  // A STALE-generation ack (the pre-handover 41) is decoded and dispatched but
  // must NOT confirm recovery.
  push(taggedFirstChunk(...linkLossClearedLD(7, MOTION_SCOPE, 41)));
  await tick();
  check("a stale-generation ack does NOT resume", state.motionRecovered === false);

  // An ack for the GIMBAL scope on the current generation must not resume
  // motion either.
  push(linkLossClearedLD(7, "vehicle.gimbal", 42));
  await tick();
  check("a gimbal-scope ack does NOT resume motion", state.motionRecovered === false);

  // Finally the matching ack (motion scope, generation 42) resumes — once.
  push(linkLossClearedLD(7, MOTION_SCOPE, 42));
  await tick();
  check("the matching-generation motion ack resumes", state.motionRecovered === true);
  check("exactly one confirmation, on generation 42", confirmed.length === 1 && confirmed[0] === 42n);

  close();
  await done;
})();

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all uni-stream checks passed");
