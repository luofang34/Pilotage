// Conformance between the wasm wire decode (compiled from the host's Rust
// definitions) and the hand-written JS reference decoders retained as an
// independent defense. The wasm exports are the drift-proof source of truth;
// these tests pin that the two agree on the field layout, the numeric kinds
// (u64 -> BigInt, u32 -> Number), and the capture-identity contract —
// including the honestly-unavailable mapping (target clock 0), which is
// well-formed and must never be rejected as malformed.

import { readFileSync } from "node:fs";
import { parseVideoFrameV2, decodeBareEnvelope } from "./wire.js";
import { metaFault } from "./video-identity.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

// ---- load the real wasm module (as the browser and CI build it) -------------

let decodeVideoFrameV2;
let decodeDatagramEnvelope;
try {
  const bindings = await import("./instrument-runtime.js");
  await bindings.default({
    module_or_path: readFileSync(new URL("./instrument-runtime_bg.wasm", import.meta.url)),
  });
  ({ decodeVideoFrameV2, decodeDatagramEnvelope } = bindings);
} catch (error) {
  check(`wasm wire decode loads (build it: scripts/build-web-instruments.sh) — ${error}`, false);
  process.exit(1);
}

// Deep equality that treats a bigint and its numeric-equal partner as distinct
// (so a wrong numeric kind fails), null and undefined as equal (serde None), and
// walks arrays and plain objects.
function deepEqual(a, b) {
  if (a === null || a === undefined) return b === null || b === undefined;
  if (typeof a !== typeof b) return false;
  if (typeof a === "bigint") return a === b;
  if (typeof a !== "object") return a === b || (Number.isNaN(a) && Number.isNaN(b));
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  const keys = new Set([...ka, ...kb]);
  for (const k of keys) {
    if (!deepEqual(a[k], b[k])) return false;
  }
  return true;
}

// ---- v2 video body encoder (mirrors the fixed on-wire layout) ---------------

function encodeV2Body(f) {
  const payload = f.payload ?? new Uint8Array(0);
  const buf = new Uint8Array(84 + payload.length);
  const dv = new DataView(buf.buffer);
  buf[0] = f.sourceId;
  dv.setUint32(1, f.sourceEpoch, true);
  buf.set(f.incarnation, 5);
  dv.setUint32(21, f.sequence, true);
  dv.setBigUint64(25, f.captureTime, true);
  buf[33] = f.captureClock;
  buf[34] = f.mappingAvailable ? 1 : 0;
  buf[35] = f.mappingTargetClock;
  dv.setBigInt64(36, f.mappingOffset, true);
  dv.setBigUint64(44, f.clockErrorBound, true);
  dv.setBigUint64(52, f.receiveTime, true);
  dv.setBigUint64(60, f.publicationTime, true);
  dv.setUint32(68, f.cameraId, true);
  dv.setUint32(72, f.calibrationId, true);
  for (let i = 0; i < 4; i += 1) buf[76 + i] = f.fourcc.charCodeAt(i);
  dv.setUint32(80, payload.length, true);
  buf.set(payload, 84);
  return buf;
}

const baseFrame = {
  sourceId: 0,
  sourceEpoch: 4,
  incarnation: new Uint8Array(16).fill(0xab),
  sequence: 12345,
  captureTime: 958_156_000_000n,
  captureClock: 2,
  mappingAvailable: true,
  mappingTargetClock: 2,
  mappingOffset: -1000n,
  clockErrorBound: 250n,
  receiveTime: 940_306_043_166n,
  publicationTime: 940_314_019_958n,
  cameraId: 0,
  calibrationId: 0,
  fourcc: "MJPG",
  payload: new Uint8Array([0xff, 0xd8, 1, 2, 3, 0xff, 0xd9]),
};

// A bounded mapping decodes identically to the JS reference, with no fault.
{
  const body = encodeV2Body(baseFrame);
  const wasm = decodeVideoFrameV2(body);
  const ref = parseVideoFrameV2(body);
  check("v2 bounded: wasm meta equals JS reference meta", deepEqual(wasm.meta, ref.meta));
  check("v2 bounded: fourcc matches", wasm.fourcc === ref.fourcc);
  check("v2 bounded: fault is null (contract satisfied)", wasm.fault === null || wasm.fault === undefined);
  check("v2 bounded: JS metaFault agrees (null)", metaFault(ref.meta) === null);
  const payload = body.subarray(wasm.payloadOffset, wasm.payloadOffset + wasm.payloadLen);
  check("v2 bounded: payload slice round-trips", deepEqual([...payload], [...ref.payload]));
  check("v2 bounded: u64 fields are BigInt", typeof wasm.meta.captureTimeNanos === "bigint");
  check("v2 bounded: u32 fields are Number", typeof wasm.meta.sourceEpoch === "number");
}

// The unavailable-mapping case (target clock 0) is well-formed: a producer
// with no capture-clock mapping (aviate publishes exactly this shape) must
// decode cleanly, never as malformed.
{
  const body = encodeV2Body({
    ...baseFrame,
    mappingAvailable: false,
    mappingTargetClock: 0,
    mappingOffset: 0n,
    clockErrorBound: 0n,
  });
  const wasm = decodeVideoFrameV2(body);
  check("v2 unavailable mapping: wasm reports NO fault", wasm.fault === null || wasm.fault === undefined);
  check("v2 unavailable mapping: JS metaFault agrees (null)", metaFault(wasm.meta) === null);
  check("v2 unavailable mapping: mappingAvailable is false", wasm.meta.mappingAvailable === false);
  check("v2 unavailable mapping: mappingTargetClock is 0", wasm.meta.mappingTargetClock === 0);
}

// A non-canonical mapping_available octet (2) is refused by both paths before
// it can be normalized to "unavailable" (offset 34 in the fixed header).
{
  const body = encodeV2Body({ ...baseFrame, mappingAvailable: false, mappingTargetClock: 0 });
  body[34] = 2;
  check("v2 non-canonical mapping_available: wasm decodes to null", decodeVideoFrameV2(body) === null);
  check("v2 non-canonical mapping_available: JS reference decodes to null", parseVideoFrameV2(body) === null);
}

// A present mapping naming clock 0 is a contract violation on both paths.
{
  const body = encodeV2Body({ ...baseFrame, mappingAvailable: true, mappingTargetClock: 0 });
  const wasm = decodeVideoFrameV2(body);
  check(
    "v2 present mapping w/ target 0: wasm faults on mappingTargetClock",
    wasm.fault && wasm.fault.field === "mappingTargetClock",
  );
  check(
    "v2 present mapping w/ target 0: JS metaFault agrees",
    metaFault(wasm.meta)?.field === "mappingTargetClock",
  );
}

// An unknown capture clock faults on both paths.
{
  const body = encodeV2Body({ ...baseFrame, captureClock: 7 });
  const wasm = decodeVideoFrameV2(body);
  check("v2 unknown capture clock: wasm faults on captureClock", wasm.fault?.field === "captureClock");
  check("v2 unknown capture clock: JS metaFault agrees", metaFault(wasm.meta)?.field === "captureClock");
}

// A structurally short body decodes to null on both paths.
{
  const wasm = decodeVideoFrameV2(new Uint8Array(40));
  check("v2 short body: wasm decodes to null", wasm === null);
  check("v2 short body: JS reference decodes to null", parseVideoFrameV2(new Uint8Array(40)) === null);
}

// ---- minimal protobuf encoder for a telemetry Envelope ----------------------

function varint(n) {
  const out = [];
  let v = BigInt(n);
  do {
    let b = Number(v & 0x7fn);
    v >>= 7n;
    if (v) b |= 0x80;
    out.push(b);
  } while (v);
  return out;
}
function tag(field, wireType) {
  return varint((field << 3) | wireType);
}
function lenField(field, bytes) {
  return [...tag(field, 2), ...varint(bytes.length), ...bytes];
}
function varintField(field, n) {
  return [...tag(field, 0), ...varint(n)];
}
function floatField(field, value) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, value, true);
  return [...tag(field, 5), ...b];
}

function stampBytes(s) {
  return [
    ...varintField(1, s.sourceId),
    ...varintField(2, s.sourceEpoch),
    ...varintField(3, s.sequence),
    ...varintField(4, s.acquiredAtNanos),
    ...varintField(5, s.clock),
    ...lenField(6, s.incarnation),
  ];
}

// A telemetry sample with pose, velocity, and an avionics attitude group.
const stamp = { sourceId: 7, sourceEpoch: 3, sequence: 9, acquiredAtNanos: 123456, clock: 2, incarnation: new Array(16).fill(0xab) };
const avionicsBytes = [
  ...floatField(1, 1.0), ...floatField(2, 0.1), ...floatField(3, 0.2), ...floatField(4, 0.3),
  ...floatField(5, 0.4), ...floatField(6, 0.5), ...floatField(7, 0.6),
  ...varintField(14, 0x0f), ...varintField(15, 0), ...varintField(16, 1),
  ...lenField(17, stampBytes(stamp)),
];
const poseBytes = [...floatField(1, 2.5), ...floatField(2, -1.5), ...floatField(3, 0.75)];
const velocityBytes = [...floatField(1, 4.0), ...floatField(2, 0.0), ...floatField(3, 0.1)];
const sampleBytes = [
  ...lenField(1, varintField(1, 1)),        // vehicle { value: 1 }
  ...lenField(2, varintField(1, 42)),       // tick { value: 42 }
  ...lenField(3, varintField(1, 900)),      // observed_at { nanos: 900 }
  ...lenField(4, poseBytes),
  ...lenField(5, velocityBytes),
  ...lenField(6, avionicsBytes),
];
const envelopeBytes = new Uint8Array([...varintField(1, 1), ...lenField(4, sampleBytes)]);

// The wasm telemetry decode agrees with the JS reference on the full shape,
// including BigInt/Number kinds and the flattened avionics groups.
{
  const wasm = decodeDatagramEnvelope(envelopeBytes);
  const ref = decodeBareEnvelope(envelopeBytes);
  check("telemetry: wasm kind is TelemetrySample", wasm.kind === "TelemetrySample");
  check("telemetry: kind matches reference", wasm.kind === ref.kind);
  check("telemetry: vehicleId is BigInt(1)", wasm.message.vehicleId === 1n);
  check("telemetry: vehicleId matches reference", wasm.message.vehicleId === ref.message.vehicleId);
  check("telemetry: tick matches reference", wasm.message.tick === ref.message.tick);
  check("telemetry: publishedAtNanos matches reference", wasm.message.publishedAtNanos === ref.message.publishedAtNanos);
  check("telemetry: pose matches reference", deepEqual(wasm.message.pose, ref.message.pose));
  check("telemetry: velocity matches reference", deepEqual(wasm.message.velocity, ref.message.velocity));
  check("telemetry: avionics matches reference", deepEqual(wasm.message.avionics, ref.message.avionics));
  check("telemetry: attitude stamp sourceId is BigInt", typeof wasm.message.avionics.attitudeStamp.sourceId === "bigint");
  check("telemetry: attitude stamp incarnation is 32 hex", /^[0-9a-f]{32}$/.test(wasm.message.avionics.attitudeStamp.sourceIncarnation));
  check("telemetry: kinematics absent (no stamp) is nullish", wasm.message.avionics.kinematics === null || wasm.message.avionics.kinematics === undefined);
}

// ---- H.264 chunk classification (shared pilotage_protocol::h264) ------------
// The recorded encoder fixture and synthetic faults classify identically to
// the Rust unit tests: same kinds, same codec string, same typed reasons.
{
  const { classifyH264Chunk } = await import("./instrument-runtime.js");
  const fixture = new Uint8Array(
    readFileSync(new URL("../../crates/pilotage-protocol/tests/fixtures/h264-annexb-baseline.h264", import.meta.url)),
  );
  const cls = classifyH264Chunk(fixture);
  check("h264: recorded fixture is a decodable keyframe", cls.kind === "keyframe");
  check("h264: recorded fixture names the libx264 baseline codec", cls.codec === "avc1.42c00a");
  const nal = (type, ...body) => [0, 0, 0, 1, type & 0x1f, ...body];
  const noPps = new Uint8Array([...nal(7, 0x42, 0xe0, 0x1e), ...nal(5, 1)]);
  const bad = classifyH264Chunk(noPps);
  check("h264: keyframe without PPS is undecodable with the typed reason", bad.kind === "undecodable-keyframe" && bad.reason === "no in-band PPS precedes the IDR");
  const late = classifyH264Chunk(new Uint8Array([...nal(5, 1), ...nal(7, 0x42, 0xe0, 0x1e), ...nal(8, 1)]));
  check("h264: parameter sets after the IDR do not configure it", late.kind === "undecodable-keyframe" && late.reason === "no in-band SPS precedes the IDR");
  const delta = classifyH264Chunk(new Uint8Array(nal(1, 7)));
  check("h264: non-IDR access unit is delta", delta.kind === "delta");
  check("h264: bytes with no NAL units are invalid, not delta", classifyH264Chunk(new Uint8Array([9, 9, 9])).kind === "invalid");
}

// A pong datagram decodes to the Pong kind.
{
  const pong = new Uint8Array([...varintField(1, 1), ...lenField(11, [])]);
  const wasm = decodeDatagramEnvelope(pong);
  check("pong: wasm kind is Pong", wasm.kind === "Pong");
}

console.log(failures === 0 ? "\nall wasm wire-decode conformance checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
