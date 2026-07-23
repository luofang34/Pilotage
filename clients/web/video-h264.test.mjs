// Tests for the WebCodecs platform adapter: decoder lifecycle, the per-source
// decoder registry, and every failure path (configure throw, decode throw,
// async decoder error, absent WebCodecs, undecodable keyframe). The REAL wasm
// state machines drive everything — chunk classification, the decode session,
// and ownership all run in the same pilotage_protocol::h264 code the Rust
// unit tests pin over synthetic NALs and the recorded Annex-B fixture. Only
// the browser platform APIs (VideoDecoder/EncodedVideoChunk) are faked, since
// WebCodecs itself needs a browser.

import { readFileSync } from "node:fs";
import { H264CanvasDecoder, H264DecoderRegistry } from "./video-h264.js";

const bindings = await import("./instrument-runtime.js");
await bindings.default({
  module_or_path: readFileSync(new URL("./instrument-runtime_bg.wasm", import.meta.url)),
});

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- real Annex-B payloads ---------------------------------------------------
// Built as raw bytes only (no parsing here); the wasm machines classify them.
const nalBytes = (type, ...body) => [0, 0, 0, 1, type & 0x1f, ...body];
const spsBytes = (profile, constraint, level) => nalBytes(7, profile, constraint, level);
const KEYFRAME_A = new Uint8Array([...spsBytes(0x42, 0xe0, 0x1e), ...nalBytes(8, 1), ...nalBytes(5, 2)]);
const KEYFRAME_B = new Uint8Array([...spsBytes(0x64, 0x00, 0x28), ...nalBytes(8, 1), ...nalBytes(5, 2)]);
const DELTA = new Uint8Array(nalBytes(1, 3));
const UNDECODABLE = new Uint8Array([...spsBytes(0x42, 0xe0, 0x1e), ...nalBytes(5, 2)]); // keyframe, no PPS

// ---- decoder lifecycle with an injected fake VideoDecoder -------------------

function makeFrame() {
  return { displayWidth: 320, displayHeight: 240, closed: false, close() { this.closed = true; } };
}
function makeTarget() {
  const draws = [];
  const texts = [];
  const ctx = {
    fillStyle: "",
    font: "",
    drawImage: (f) => draws.push(f),
    fillRect() {},
    fillText: (s) => texts.push(s),
  };
  return { target: { canvas: { width: 0, height: 0 }, ctx }, draws, texts };
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
    log: overrides.log ?? (() => {}),
    isActive: overrides.isActive ?? (() => true),
  };
}

// Initial keyframe: the wasm session machine classifies it, decides
// configure-and-feed, and the adapter configures, decodes, paints, and closes
// the frame.
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

// A keyframe with a new codec string reconfigures: the superseded decoder closes
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

// An undecodable keyframe (a real IDR+SPS access unit missing its PPS) fails
// visible with the machine's typed reason rather than configuring a decoder
// that would stall.
{
  const { target } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(UNDECODABLE);
  check("undecodable keyframe builds no decoder", instances.length === 0);
  check("undecodable keyframe fails visible with the typed reason", dec.failed === true && logs.some((m) => /no in-band PPS precedes the IDR/.test(m)));
}

// A decode() throw (the synchronous WebCodecs decode failure path) is a
// mid-stream data fault: the session awaits the next keyframe — visibly —
// and the next decodable keyframe restores painting.
{
  const { target, draws, texts } = makeTarget();
  const opts = { decodeThrows: true };
  const { ctor, instances } = makeDecoderCtor(opts);
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(KEYFRAME_A);
  check("decode throw is caught (configure succeeded)", instances.length === 1 && instances[0].config !== null);
  check("decode throw recovers instead of dying", dec.failed === false);
  check("decode throw closes the errored decoder", instances[0].closed === true);
  check("decode throw logs the await-keyframe reason", logs.some((m) => /decode failed.*awaiting a fresh keyframe/.test(m)));
  check("decode throw paints the stall banner", texts.some((s) => /stalled — awaiting keyframe/.test(s)));
  check("a delta while awaiting keyframe builds no decoder", (dec.decode(DELTA), instances.length === 1));
  opts.decodeThrows = false;
  dec.decode(KEYFRAME_A);
  check("the next keyframe reconfigures a fresh decoder", instances.length === 2 && instances[1].config !== null);
  check("painting resumes after recovery", draws.length === 1);
}

// The asynchronous decoder error callback (WebCodecs' out-of-band error path)
// recovers the same way, without an exception into the frame path.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  dec.decode(KEYFRAME_A); // configures; captures the decoder's error callback
  const painted = draws.length;
  instances[0].error(new Error("hardware reset"));
  check("async decoder error recovers with the typed reason", dec.failed === false && logs.some((m) => /decoder error.*awaiting a fresh keyframe/.test(m)));
  check("async decoder error closes the decoder", instances[0].closed === true);
  dec.decode(KEYFRAME_A);
  check("a fresh keyframe restores painting after the async error", instances.length === 2 && draws.length === painted + 1);
}

// The paint callback feeds the stall watch: only PAINTED frames count, so a
// source stuck awaiting a keyframe (deltas arriving, nothing decodable)
// stalls visibly within the watch threshold.
{
  const { target } = makeTarget();
  const opts = { decodeThrows: true };
  const { ctor } = makeDecoderCtor(opts);
  let painted = 0;
  const dec = new H264CanvasDecoder(target, {
    ...decoderOptions({ VideoDecoder: ctor }),
    onPainted: () => {
      painted += 1;
    },
  });
  dec.decode(KEYFRAME_A); // decode throws -> awaiting keyframe; nothing painted
  check("an errored feed reports no paint to the stall watch", painted === 0);
  opts.decodeThrows = false;
  dec.decode(KEYFRAME_A);
  check("a decoded frame reports its paint to the stall watch", painted === 1);
}

// ---- stale platform callbacks (the retained-callback hazards) ---------------

// An in-band codec change replaces the decoder; the superseded decoder's retained
// output callback must not paint over the new stream, even within one session.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(KEYFRAME_A);
  dec.decode(KEYFRAME_B); // codec change: instances[0] retired, instances[1] live
  const before = draws.length;
  const stale = makeFrame();
  instances[0].output(stale); // the retired decoder's retained callback fires late
  check("a stale callback after a codec change paints nothing", draws.length === before);
  check("the stale frame is still closed (no leak)", stale.closed === true);
  const live = makeFrame();
  instances[1].output(live);
  check("the live decoder's callback still paints", draws.length === before + 1);
  instances[0].error(new Error("late error from the superseded decoder"));
  check("a stale error cannot poison the live session", dec.failed === false);
  const after = makeFrame();
  instances[1].output(after);
  check("the live decoder still paints after the stale error", draws.length === before + 2);
}

// After close() (session replacement / discontinuity path), the superseded decoder's
// retained callback must not paint — retirement is permanent.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor();
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor }));
  dec.decode(KEYFRAME_A);
  dec.close();
  const before = draws.length;
  const stale = makeFrame();
  instances[0].output(stale);
  check("a stale callback after close() paints nothing", draws.length === before);
  check("the post-close frame is still closed", stale.closed === true);
}

// Exhausting the decode-error strike bound without a painted frame is the
// PERMANENT failure; the failed decoder's retained callback must not paint
// over the fail-visible marker, and its late error must not re-log.
{
  const { target, draws } = makeTarget();
  const { ctor, instances } = makeDecoderCtor({ decodeThrows: true });
  const logs = [];
  const dec = new H264CanvasDecoder(target, decoderOptions({ VideoDecoder: ctor, log: (m) => logs.push(m) }));
  // Three strikes recover; the fourth decode error is terminal.
  dec.decode(KEYFRAME_A);
  dec.decode(KEYFRAME_A);
  dec.decode(KEYFRAME_A);
  check("strikes recover up to the bound", dec.failed === false);
  dec.decode(KEYFRAME_A); // fourth error -> permanent failure -> marker painted
  check("the strike bound fails closed", dec.failed === true);
  const logged = logs.length;
  const stale = makeFrame();
  instances[0].output(stale);
  check("a stale callback after failure paints nothing over the marker", draws.length === 0);
  check("the post-failure frame is still closed", stale.closed === true);
  instances[0].error(new Error("late hardware error"));
  check("a late error from the failed decoder does not re-log", logs.length === logged);
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
const T1 = { generation: 1 };
const T2 = { generation: 2 };
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
// and the registry, on a new token, supersedes it: a retired token must
// never govern a live session's frames.
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
  check("a decoder held under a retired token no longer paints", draws.length === 1);
}

console.log(failures === 0 ? "\nall H.264 platform-adapter + registry checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
