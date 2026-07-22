// The live authority-stream path: a LinkLossCleared recovery ack arrives
// FRAGMENTED while the stream is still open, must be decoded and dispatched the
// instant it completes (never buffered until close).

import { drainAuthorityEnvelopes } from "./authority-stream.js";
import { decodeLengthDelimitedEnvelope } from "./wire.js";

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

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
function appendChunk(a, b) {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

// A length-delimited Envelope carrying a LinkLossCleared (oneof tag 15).
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

function leaseGrantedLD(principal, vehicle, scope, generation) {
  const granted = [];
  bytesField(granted, 1, uint64Message(principal));
  bytesField(granted, 2, uint64Message(vehicle));
  bytesField(granted, 3, stringMessage(scope));
  bytesField(granted, 4, uint64Message(generation));
  const event = [];
  bytesField(event, 1, granted);
  const envelope = [];
  bytesField(envelope, 3, event);
  const ld = [];
  varint(ld, envelope.length);
  ld.push(...envelope);
  return new Uint8Array(ld);
}

// ---- fragmented arrival on an OPEN stream ----
{
  const bytes = linkLossClearedLD(7, "vehicle.motion", 42);
  const dispatched = [];
  const onEnv = (d) => dispatched.push(d);

  // First chunk carries only part of the envelope: nothing dispatches yet, the
  // partial bytes are held over (the stream has NOT closed).
  let buf = drainAuthorityEnvelopes(bytes.subarray(0, 4), decodeLengthDelimitedEnvelope, onEnv);
  check("a fragmented envelope dispatches nothing yet", dispatched.length === 0);
  check("the partial bytes are buffered for the next chunk", buf.length === 4);

  // The remainder arrives: the envelope completes and dispatches LIVE, without
  // waiting for the stream to close.
  buf = drainAuthorityEnvelopes(appendChunk(buf, bytes.subarray(4)), decodeLengthDelimitedEnvelope, onEnv);
  check("the completed envelope dispatches live", dispatched.length === 1);
  check("it decoded as LinkLossCleared", dispatched[0]?.kind === "LinkLossCleared");
  check("no leftover after a complete envelope", buf.length === 0);
  check("the generation round-tripped", dispatched[0]?.message.generation === 42n);
}

// ---- broadcast grants carry the identity needed for table dedup ----------
{
  const decoded = decodeLengthDelimitedEnvelope(
    leaseGrantedLD(5, 7, "vehicle.motion", 42),
  );
  check("a grant exposes its authority kind", decoded?.message.kind === "grant");
  check("a grant exposes its principal", decoded?.message.principalId === 5n);
  check("a grant exposes its vehicle", decoded?.message.vehicleId === 7n);
  check("a grant exposes its scope", decoded?.message.scope === "vehicle.motion");
  check("a grant exposes its generation", decoded?.message.generation === 42n);
}

// ---- two back-to-back envelopes in one chunk both dispatch ----
{
  const a = linkLossClearedLD(7, "vehicle.motion", 42);
  const b = linkLossClearedLD(7, "vehicle.gimbal", 43);
  const dispatched = [];
  const buf = drainAuthorityEnvelopes(appendChunk(a, b), decodeLengthDelimitedEnvelope, (d) => dispatched.push(d));
  check("two coalesced envelopes both dispatch", dispatched.length === 2);
  check("no leftover after two complete envelopes", buf.length === 0);
}

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all authority-stream checks passed");
