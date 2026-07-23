// The PRODUCTION long-lived uni-stream reader — `readUniStream`, the core of
// main.js's `readOneUniStream` — driven end to end against a REAL
// ReadableStream, together with the real wasm authority transition. This
// exercises what the helper-only test cannot: the
// leading 0x01 authority kind tag, a LinkLossCleared arriving FRAGMENTED on an
// OPEN (never-closing) stream, and the recovered authority flag flipping LIVE —
// before any close — and ONLY for the matching generation.
//
// Synchronization is deterministic, NOT timer-based: a PULL-driven stream calls
// its `pull` callback exactly when the reader has consumed the previous chunk
// and is asking for the next, so each callback is a precise checkpoint — no
// setTimeout races.
//
// Run: node clients/web/uni-stream.test.mjs

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";
import { readUniStream } from "./uni-stream.js";
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

// A PULL-driven ReadableStream: `pull` runs one scripted step each time the
// reader asks for the next chunk (i.e. after fully consuming the previous one),
// so a step is a deterministic checkpoint. The stream stays OPEN until a step
// calls `controller.close()`.
function scriptedStream(steps) {
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < steps.length) {
        steps[i](controller);
        i += 1;
      } else {
        controller.close();
      }
    },
  });
}

function startReader(reader, shell, vehicleId, motionScope) {
  return readUniStream(reader, {
    authorityKind: STREAM_KIND_AUTHORITY,
    decode: decodeLengthDelimitedEnvelope,
    onAuthorityEnvelope: (decoded) => {
      if (
        decoded.kind === "LinkLossCleared" &&
        decoded.message.vehicleId === vehicleId &&
        decoded.message.scope === motionScope
      ) {
        shell.authorityEvent("motion", "recovery", {
          generation: decoded.message.generation,
        });
      }
    },
  });
}

const wasmBytes = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));

async function recoveringShell() {
  const shell = await loadControlShell(wasmBytes);
  shell.beginSession();
  shell.authorityEvent("motion", "grant", { generation: 41n });
  shell.authorityEvent("motion", "release", { generation: 41n });
  shell.authorityEvent("motion", "grant", { generation: 42n });
  return shell;
}

const VEHICLE_ID = 7n;
const MOTION_SCOPE = "vehicle.motion";

// ---- 0x01 tag + fragmented OPEN stream + LIVE recovery transition ----
await (async () => {
  const shell = await recoveringShell();
  const ack = linkLossClearedLD(7, MOTION_SCOPE, 42);
  // Observations are recorded INSIDE the pull checkpoints, so they capture the
  // recovery latch at exact points in the reader's progress.
  const observed = [];
  const stream = scriptedStream([
    // Deliver the kind tag and only the first bytes of the ack.
    (c) => c.enqueue(taggedFirstChunk(...ack.subarray(0, 3))),
    // Reader consumed the partial (asking for more): nothing recovered yet;
    // deliver the remainder — still on the OPEN stream.
    (c) => {
      observed.push(["after-partial", shell.authority("motion").recovered]);
      c.enqueue(ack.subarray(3));
    },
    // Reader consumed the completing bytes: the ack dispatched and recovery
    // latched LIVE — recorded BEFORE we close the stream.
    (c) => {
      observed.push(["after-complete-open", shell.authority("motion").recovered]);
      c.close();
    },
  ]);

  const { kind, tail, aborted } = await startReader(
    stream.getReader(),
    shell,
    VEHICLE_ID,
    MOTION_SCOPE,
  );

  check("a partial envelope on an open stream does not recover", observed[0][1] === false);
  check(
    "recovery latched LIVE from the fragmented OPEN stream (before close)",
    observed[1][1] === true,
  );
  check("the leading 0x01 tag was read as the authority kind", kind === STREAM_KIND_AUTHORITY);
  check("no video tail is left on the authority stream", tail.length === 0);
  check("the reader was not aborted", aborted === false);
})();

// ---- resumes ONLY the matching generation, through the real reader ----
await (async () => {
  const shell = await recoveringShell();
  const observed = [];
  const stream = scriptedStream([
    // A STALE-generation ack (the pre-handover 41) — decoded and dispatched.
    (c) => c.enqueue(taggedFirstChunk(...linkLossClearedLD(7, MOTION_SCOPE, 41))),
    // Recorded after the stale ack: must NOT resume. Deliver a GIMBAL-scope ack.
    (c) => {
      observed.push(["after-stale", shell.authority("motion").recovered]);
      c.enqueue(linkLossClearedLD(7, "vehicle.gimbal", 42));
    },
    // Recorded after the gimbal ack: must NOT resume motion. Deliver the match.
    (c) => {
      observed.push(["after-gimbal", shell.authority("motion").recovered]);
      c.enqueue(linkLossClearedLD(7, MOTION_SCOPE, 42));
    },
    // Recorded after the matching ack: NOW resumed.
    (c) => {
      observed.push(["after-match", shell.authority("motion").recovered]);
      c.close();
    },
  ]);

  await startReader(stream.getReader(), shell, VEHICLE_ID, MOTION_SCOPE);

  check("a stale-generation ack does NOT resume", observed[0][1] === false);
  check("a gimbal-scope ack does NOT resume motion", observed[1][1] === false);
  check("the matching-generation motion ack resumes", observed[2][1] === true);
})();

// ---- mid-read abandonment returns the stream to its producer ----
await (async () => {
  let cancellation = null;
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([2, 1, 2, 3]));
    },
    cancel(reason) {
      cancellation = reason;
    },
  });
  const result = await readUniStream(stream.getReader(), {
    authorityKind: STREAM_KIND_AUTHORITY,
    decode: decodeLengthDelimitedEnvelope,
    onAuthorityEnvelope: () => {},
    shouldContinue: () => false,
  });
  check("an abandoned mid-read stream reports aborted", result.aborted === true);
  check(
    "an abandoned mid-read stream is cancelled with a typed reason",
    cancellation?.name === "StreamCancellationReason" && cancellation?.kind === "stream-abandoned",
  );
})();

// ---- decoder failure cancels rather than stranding the readable stream ----
await (async () => {
  let cancellation = null;
  const decodeFailure = new Error("injected envelope failure");
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([STREAM_KIND_AUTHORITY, 0]));
    },
    cancel(reason) {
      cancellation = reason;
    },
  });
  let rejected = null;
  try {
    await readUniStream(stream.getReader(), {
      authorityKind: STREAM_KIND_AUTHORITY,
      decode: () => {
        throw decodeFailure;
      },
      onAuthorityEnvelope: () => {},
    });
  } catch (error) {
    rejected = error;
  }
  check("the original stream failure remains the rejected error", rejected === decodeFailure);
  check(
    "a failed stream drain cancels with the failure preserved as its cause",
    cancellation?.kind === "stream-read-failed" && cancellation?.cause === decodeFailure,
  );
})();

// ---- a cancellation failure is observable without masking the read failure ----
await (async () => {
  const readFailure = new Error("injected read failure");
  const cancelFailure = new Error("injected cancel failure");
  let observed = null;
  const reader = {
    read: () => Promise.reject(readFailure),
    cancel: () => Promise.reject(cancelFailure),
  };
  try {
    await readUniStream(reader, {
      authorityKind: STREAM_KIND_AUTHORITY,
      decode: decodeLengthDelimitedEnvelope,
      onAuthorityEnvelope: () => {},
      onCancelFailure: (error, reason) => {
        observed = { error, reason };
      },
    });
  } catch (error) {
    check("cancellation failure does not mask the read failure", error === readFailure);
  }
  check(
    "cancellation failure is reported through the diagnostic seam",
    observed?.error === cancelFailure && observed?.reason.kind === "stream-read-failed",
  );
})();

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all uni-stream checks passed");
