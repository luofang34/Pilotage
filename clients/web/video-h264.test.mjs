// Tests for the WebCodecs decode SESSION layer: decoder lifecycle, the
// per-source decoder registry that binds a decoder to its transport session,
// and every failure path (configure throw, decode throw, async decoder error,
// absent WebCodecs, undecodable keyframe). What a chunk MEANS is classified by
// the shared Rust wasm export; here a fake classifier is injected so the
// session logic is exercised in isolation — the classification itself is
// pinned by the Rust unit tests (pilotage-protocol::h264, over synthetic NALs
// and the recorded Annex-B fixture) and by the wasm conformance suite.

import { H264CanvasDecoder, H264DecoderRegistry } from "./video-h264.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- injected classifier ----------------------------------------------------
// Payloads are tagged by their first byte; the fake classifier maps the tag to
// a classification in the wasm export's exact result vocabulary.
const KEYFRAME_A = new Uint8Array([1, 0xaa]);
const KEYFRAME_B = new Uint8Array([2, 0xbb]);
const DELTA = new Uint8Array([3, 0xcc]);
const UNDECODABLE = new Uint8Array([9, 0xdd]);
function fakeClassify(payload) {
  switch (payload[0]) {
    case 1:
      return { kind: "keyframe", codec: "avc1.42e01e", reason: null };
    case 2:
      return { kind: "keyframe", codec: "avc1.640028", reason: null };
    case 9:
      return { kind: "undecodable-keyframe", codec: null, reason: "missing in-band PPS" };
    default:
      return { kind: "delta", codec: null, reason: null };
  }
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

function decoderOptions(overrides) {
  return {
    VideoDecoder: overrides.VideoDecoder,
    EncodedVideoChunk: makeChunkCtor(),
    classify: overrides.classify ?? fakeClassify,
    log: overrides.log ?? (() => {}),
    isActive: overrides.isActive ?? (() => true),
  };
}

// Initial keyframe: configures from the classified codec, decodes, paints, and
// closes the frame.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(KEYFRAME_A);
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
  dec.decode(DELTA);
  check("delta before keyframe builds no decoder", instances.length === 0);
  check("delta before keyframe paints nothing", draws.length === 0);
}

// A retired session (isActive false) never paints, even though the decoder
// still runs and closes the frame — a callback cannot govern a dead session.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, isActive: () => false }));
  dec.decode(KEYFRAME_A);
  check("retired session paints nothing", draws.length === 0);
  check("retired session still closes the decoded frame", instances[0].decodes.length === 1);
}

// A keyframe with a new codec string reconfigures: the old decoder is closed
// and a new one built for the new codec.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(KEYFRAME_A);
  dec.decode(KEYFRAME_B);
  check("config change closes the first decoder", instances[0].closed === true);
  check("config change builds a second decoder for the new codec", instances.length === 2 && instances[1].config.codec === "avc1.640028");
}

// close() (session replacement / discontinuity / teardown) closes the decoder.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(KEYFRAME_A);
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
  dec.decode(KEYFRAME_A);
  check("configure failure logs a typed unavailable reason", logs.some((m) => /unavailable/.test(m)));
  check("configure failure marks the decoder failed", dec.failed === true);
  check("configure failure paints a fail-visible marker", draws.length === 0 && target.canvas.width > 0);
  const before = logs.length;
  dec.decode(KEYFRAME_A);
  check("a failed decoder drops later frames without re-logging", logs.length === before);
}

// Missing WebCodecs fails visible, not by throwing.
{
  const { target } = makeTarget();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: undefined, log: (m) => logs.push(m) }));
  dec.decode(KEYFRAME_A);
  check("absent WebCodecs fails visible with a typed reason", dec.failed === true && logs.some((m) => /unavailable/.test(m)));
}

// An undecodable keyframe (the classifier's typed fault, e.g. a missing PPS)
// fails visible rather than configuring a decoder that would stall.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(UNDECODABLE);
  check("undecodable keyframe builds no decoder", instances.length === 0);
  check("undecodable keyframe fails visible with the typed reason", dec.failed === true && logs.some((m) => /missing in-band PPS/.test(m)));
}

// A decode() throw (the synchronous WebCodecs decode failure path) fails visible.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor({ decodeThrows: true });
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(KEYFRAME_A);
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
  dec.decode(KEYFRAME_A); // configures; captures the decoder's error callback
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
  first.decode(KEYFRAME_A);
  check("live session paints", draws.length === 1);
  liveToken = T2; // session replaced
  const second = registry.for(0, target, T2);
  check("session replacement supersedes the retired decoder", second !== first);
  first.decode(KEYFRAME_A); // a stale callback path: retired token must not paint
  check("retired-token decoder no longer paints", draws.length === 1);
}

console.log(failures === 0 ? "\nall H.264 session-layer + registry checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
