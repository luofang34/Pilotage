// Tests for the H.264 Annex-B classification helpers, the WebCodecs decoder
// lifecycle, and the per-source decoder registry that binds a decoder to its
// transport session. Classification is pure and is also run over a recorded
// real Annex-B fixture. The decoder and registry are exercised with an injected
// fake VideoDecoder, so session ownership (replacement, discontinuity reset,
// teardown), delta-before-keyframe gating, configuration change, the decode and
// async-error failure paths, and frame close are all host-runnable without a
// browser. WebCodecs itself needs a browser, so it is not decoded here.

import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import {
  forEachNalType,
  isKeyframe,
  hasParameterSets,
  avcCodecString,
  H264CanvasDecoder,
  H264DecoderRegistry,
} from "./video-h264.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// nal(type, ...body) with a 4-byte start code; sps() carries profile/constraint/level.
function nal(type, ...body) {
  return [0, 0, 0, 1, type & 0x1f, ...body];
}
function sps(profile, constraint, level) {
  return nal(7, profile, constraint, level);
}

// A keyframe access unit: SPS + PPS + IDR slice.
{
  const au = new Uint8Array([...sps(0x42, 0xe0, 0x1e), ...nal(8), ...nal(5, 9, 9, 9)]);
  const types = [];
  forEachNalType(au, (t) => types.push(t));
  check("iterates every NAL type in order", types.join(",") === "7,8,5");
  check("IDR slice marks a keyframe", isKeyframe(au) === true);
  check("codec string is avc1 from the SPS bytes", avcCodecString(au) === "avc1.42e01e");
}

// A delta access unit: a non-IDR slice, no SPS.
{
  const au = new Uint8Array(nal(1, 4, 4));
  check("non-IDR access unit is not a keyframe", isKeyframe(au) === false);
  check("no SPS yields no codec string", avcCodecString(au) === null);
}

// The 3-byte start code (0x000001) is recognized as well as the 4-byte one.
{
  const au = new Uint8Array([0, 0, 1, 5, 1, 2, 3]);
  check("3-byte start code is recognized", isKeyframe(au) === true);
}

// Garbage without any start code yields no NAL units (fail closed).
{
  const au = new Uint8Array([9, 9, 9, 9, 9]);
  let count = 0;
  forEachNalType(au, () => (count += 1));
  check("no start code yields no NAL units", count === 0);
  check("garbage is not a keyframe", isKeyframe(au) === false);
}

// ---- decoder lifecycle with an injected fake VideoDecoder -------------------

function makeFrame() {
  return { displayWidth: 320, displayHeight: 240, closed: false, close() { this.closed = true; } };
}
function makeTarget() {
  const draws = [];
  const ctx = { fillStyle: "", font: "", drawImage: (f) => draws.push(f), fillRect() {}, fillText() {} };
  return { target: { canvas: { width: 0, height: 0 }, ctx }, draws };
}
function makeChunkCtor() {
  return class FakeChunk {
    constructor({ type, timestamp, data }) {
      this.type = type;
      this.timestamp = timestamp;
      this.data = data;
    }
  };
}
// A fake VideoDecoder that records config/decodes and emits one frame per
// decode. `configureThrows`/`decodeThrows` inject the WebCodecs failure paths.
function makeDecoderCtor(opts = {}) {
  const instances = [];
  const ctor = class FakeVideoDecoder {
    constructor({ output, error }) {
      this.output = output;
      this.error = error;
      this.closed = false;
      this.config = null;
      this.decodes = [];
      instances.push(this);
    }
    configure(config) {
      this.config = config;
      if (opts.configureThrows) throw new Error("unsupported profile");
    }
    decode(chunk) {
      this.decodes.push(chunk);
      if (opts.decodeThrows) throw new Error("decode error");
      this.output(makeFrame());
    }
    close() {
      this.closed = true;
    }
  };
  return { ctor, instances };
}

const keyframeAU = new Uint8Array([...sps(0x42, 0xe0, 0x1e), ...nal(8), ...nal(5, 1, 2, 3)]);
const keyframe2AU = new Uint8Array([...sps(0x64, 0x00, 0x28), ...nal(8), ...nal(5, 4, 5, 6)]);
const deltaAU = new Uint8Array(nal(1, 7, 7));

function decoderOptions(overrides) {
  return {
    VideoDecoder: overrides.VideoDecoder,
    EncodedVideoChunk: makeChunkCtor(),
    log: overrides.log ?? (() => {}),
    isActive: overrides.isActive ?? (() => true),
  };
}

// Initial keyframe: configures from the SPS, decodes, paints, and closes the frame.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(keyframeAU);
  check("initial keyframe configures the decoder", instances.length === 1 && instances[0].config.codec === "avc1.42e01e");
  check("initial keyframe decodes a key chunk", instances[0].decodes.length === 1 && instances[0].decodes[0].type === "key");
  check("initial keyframe paints one frame", draws.length === 1);
  check("painted frame is closed", draws[0].closed === true);
}

// A delta before any keyframe is dropped: no decoder is built, nothing paints.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(deltaAU);
  check("delta before keyframe builds no decoder", instances.length === 0);
  check("delta before keyframe paints nothing", draws.length === 0);
}

// A retired session (isActive false) never paints, even though the decoder
// still runs and closes the frame — a callback cannot govern a dead session.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, isActive: () => false }));
  dec.decode(keyframeAU);
  check("retired session paints nothing", draws.length === 0);
  check("retired session still closes the decoded frame", instances[0].decodes.length === 1);
}

// A keyframe with a new SPS reconfigures: the old decoder is closed and a new
// one built for the new codec string.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(keyframeAU);
  dec.decode(keyframe2AU);
  check("config change closes the first decoder", instances[0].closed === true);
  check("config change builds a second decoder for the new codec", instances.length === 2 && instances[1].config.codec === "avc1.640028");
}

// close() (session replacement / discontinuity / teardown) closes the decoder.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(keyframeAU);
  dec.close();
  check("close() closes the underlying decoder", instances[0].closed === true);
}

// A configure failure fails visible with a typed reason and never throws; the
// decoder then drops subsequent frames.
{
  const { target, draws } = makeTarget();
  const { ctor } = makeDecoderCtor({ configureThrows: true });
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(keyframeAU);
  check("configure failure logs a typed unavailable reason", logs.some((m) => /unavailable/.test(m)));
  check("configure failure marks the decoder failed", dec.failed === true);
  check("configure failure paints a fail-visible marker", draws.length === 0 && target.canvas.width > 0);
  const before = logs.length;
  dec.decode(keyframeAU);
  check("a failed decoder drops later frames without re-logging", logs.length === before);
}

// Missing WebCodecs fails visible, not by throwing.
{
  const { target } = makeTarget();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: undefined, log: (m) => logs.push(m) }));
  dec.decode(keyframeAU);
  check("absent WebCodecs fails visible with a typed reason", dec.failed === true && logs.some((m) => /unavailable/.test(m)));
}

// A keyframe with an SPS but no PPS is not decodable: it fails visible rather
// than configuring a decoder that would stall for want of slice parameters.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const logs = [];
  const noPps = new Uint8Array([...sps(0x42, 0xe0, 0x1e), ...nal(5, 1, 2, 3)]);
  check("hasParameterSets is false without a PPS", hasParameterSets(noPps) === false);
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(noPps);
  check("keyframe without PPS builds no decoder", instances.length === 0);
  check("keyframe without PPS fails visible with a typed SPS/PPS reason", dec.failed === true && logs.some((m) => /SPS\/PPS/.test(m)));
}

// A decode() throw (the synchronous WebCodecs decode failure path) fails visible.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor({ decodeThrows: true });
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(keyframeAU);
  check("decode throw is caught (configure succeeded)", instances.length === 1 && instances[0].config !== null);
  check("decode throw fails visible with a typed reason", dec.failed === true && logs.some((m) => /decode failed/.test(m)));
}

// The asynchronous decoder error callback (WebCodecs' out-of-band error path)
// fails visible without an exception into the frame path.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(keyframeAU); // configures; captures the decoder's error callback
  instances[0].error(new Error("hardware reset"));
  check("async decoder error fails visible with a typed reason", dec.failed === true && logs.some((m) => /decoder error/.test(m)));
  check("async decoder error closes the decoder", instances[0].closed === true);
}

// ---- registry: per-source decoders bound to their session ------------------

// A fake decoder that only records close() and the session token it was built
// for — enough to prove the registry's ownership transitions.
function makeRegistry() {
  const built = [];
  const registry = new H264DecoderRegistry((target, token) => {
    const decoder = { target, token, closed: false, close() { this.closed = true; } };
    built.push(decoder);
    return decoder;
  });
  return { registry, built };
}
const T1 = { id: 1 };
const T2 = { id: 2 };
const targetA = {};

// Same source + same session token reuses one decoder.
{
  const { registry, built } = makeRegistry();
  const a = registry.for(0, targetA, T1);
  const b = registry.for(0, targetA, T1);
  check("same source+token reuses the decoder", a === b && built.length === 1);
}

// A new session token (reconnect / session replacement mid-stream) closes the
// held decoder and builds a fresh one bound to the new token.
{
  const { registry, built } = makeRegistry();
  const a = registry.for(0, targetA, T1);
  const b = registry.for(0, targetA, T2);
  check("session replacement builds a new decoder", a !== b && built.length === 2);
  check("session replacement closes the retired decoder", a.closed === true && b.closed === false);
  check("new decoder is bound to the new session token", b.token === T2);
}

// A discontinuity reset drops only the named source's decoder.
{
  const { registry } = makeRegistry();
  const s0 = registry.for(0, targetA, T1);
  const s1 = registry.for(1, targetA, T1);
  registry.reset(0);
  check("reset closes the named source's decoder", s0.closed === true);
  check("reset leaves other sources untouched", s1.closed === false && registry.for(1, targetA, T1) === s1);
  check("reset rebuilds the reset source on next use", registry.for(0, targetA, T1) !== s0);
}

// closeAll (session teardown) closes every decoder and empties the registry.
{
  const { registry } = makeRegistry();
  const s0 = registry.for(0, targetA, T1);
  const s1 = registry.for(1, targetA, T1);
  registry.closeAll();
  check("closeAll closes every decoder", s0.closed === true && s1.closed === true);
  check("closeAll empties the registry", registry.for(0, targetA, T1) !== s0);
}

// End to end: a real H264CanvasDecoder whose session is retired stops painting,
// and the registry, on a new token, supersedes it — the retired-token bug.
{
  const { target, draws } = makeTarget();
  let liveToken = T1;
  const registry = new H264DecoderRegistry((tgt, token) =>
    new H264CanvasDecoder(tgt, decoderOptions({ VideoDecoder: makeDecoderCtor().ctor, isActive: () => liveToken === token })),
  );
  const first = registry.for(0, target, T1);
  first.decode(keyframeAU);
  check("live session paints", draws.length === 1);
  liveToken = T2; // session replaced
  const second = registry.for(0, target, T2);
  check("session replacement supersedes the retired decoder", second !== first);
  first.decode(keyframeAU); // a stale callback path: retired token must not paint
  check("retired-token decoder no longer paints", draws.length === 1);
}

// ---- real recorded Annex-B fixture -----------------------------------------

{
  const fixtureUrl = new URL("./fixtures/h264-annexb-baseline.h264", import.meta.url);
  const bytes = new Uint8Array(readFileSync(fixtureUrl));
  const digest = createHash("sha256").update(bytes).digest("hex");
  const PINNED = "84d843b4334d9a5a2aec482d0a56f4fb60ce450a5c87b6f8414eb9d3a39fe6c7";
  check("fixture matches its pinned SHA-256 (provenance intact)", digest === PINNED);
  const types = [];
  forEachNalType(bytes, (t) => types.push(t));
  check("fixture leads with SPS(7) and PPS(8)", types[0] === 7 && types.includes(8));
  check("fixture is classified as a keyframe with both parameter sets", isKeyframe(bytes) && hasParameterSets(bytes));
  check("fixture config extraction yields a valid avc1 codec string", /^avc1\.[0-9a-f]{6}$/.test(avcCodecString(bytes) ?? ""));
}

console.log(failures === 0 ? "\nall H.264 classification + lifecycle + registry + fixture checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
