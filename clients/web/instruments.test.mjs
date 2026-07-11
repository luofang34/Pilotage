// Deterministic checks for the fail-visible display pipeline (DISP-01).
//
// Run: node clients/web/instruments.test.mjs
// (build clients/web/instruments.wasm first: scripts/build-web-instruments.sh)
//
// Uses an injected clock and a command-recording canvas double — no sleeps,
// no real timers, no DOM. Covers: the failure latch and its recovery rules,
// the liveness deadline boundary, generation wrap, scene framing validation
// including truncation, transactional no-paint-on-failure, failure-page
// coverage, and the real WASM module end to end.

import { readFileSync } from "node:fs";
import {
  InstrumentModule,
  LOGICAL_H,
  LOGICAL_W,
  PANEL,
  validateSceneStructure,
} from "./instruments.js";
import { PanelHealth, REASON, drawFailurePage } from "./instrument-health.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- doubles ---------------------------------------------------------------

// Records every method call; property assignments are kept as fields so
// paint assertions can check coverage without a rasterizer.
class RecordingCtx {
  constructor() {
    this.log = [];
    const methods = [
      "save",
      "restore",
      "translate",
      "rotate",
      "setTransform",
      "beginPath",
      "moveTo",
      "lineTo",
      "closePath",
      "stroke",
      "fill",
      "fillRect",
      "strokeRect",
      "arc",
      "rect",
      "clip",
      "fillText",
      "drawImage",
    ];
    for (const m of methods) {
      this[m] = (...args) => this.log.push({ op: m, args });
    }
  }
  calls(op) {
    return this.log.filter((e) => e.op === op);
  }
}

function recordingCanvas() {
  const ctx = new RecordingCtx();
  return { width: 0, height: 0, getContext: () => ctx, ctx };
}

// A fake WASM export surface with programmable behavior.
function fakeExports({ status = 0, sceneBytes = new Uint8Array([1]), generation = () => 1 } = {}) {
  const buffer = new ArrayBuffer(65536);
  new Uint8Array(buffer).set(sceneBytes, 1024);
  return {
    abi_version: () => 2,
    init: () => 1,
    state_ptr: () => 0,
    state_len: () => 128,
    scene_ptr: () => 1024,
    scene_len: () => sceneBytes.length,
    render_status: typeof status === "function" ? status : () => status,
    render_generation: typeof generation === "function" ? generation : () => generation,
    set_v_speeds: () => {},
    memory: { buffer },
  };
}

// ---- scene byte builders ---------------------------------------------------

function f32(v) {
  const b = new ArrayBuffer(4);
  new DataView(b).setFloat32(0, v, true);
  return [...new Uint8Array(b)];
}

function cmd(op, payload) {
  return [op, payload.length & 0xff, (payload.length >> 8) & 0xff, ...payload];
}

// A small valid scene: fill-rect, save, restore. `boundaries` collects the
// byte offsets at which a prefix is still a whole number of commands.
function buildScene() {
  const parts = [
    cmd(0x23, [1, ...f32(0), ...f32(0), ...f32(10), ...f32(10)]),
    cmd(0x01, []),
    cmd(0x02, []),
  ];
  const bytes = [1];
  const boundaries = new Set([1]);
  for (const part of parts) {
    bytes.push(...part);
    boundaries.add(bytes.length);
  }
  return { bytes: Uint8Array.from(bytes), boundaries };
}

const view = (bytes) => new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

// ---- scene framing validation ----------------------------------------------

{
  const { bytes, boundaries } = buildScene();
  check("valid scene passes framing validation", validateSceneStructure(view(bytes)));
  check("empty buffer fails", !validateSceneStructure(view(new Uint8Array(0))));
  check(
    "wrong version byte fails",
    !validateSceneStructure(view(Uint8Array.from([9, ...cmd(0x01, [])]))),
  );
  const unknownOp = Uint8Array.from([1, ...cmd(0x7f, [42]), ...cmd(0x01, [])]);
  check("unknown opcode with sound framing passes (version policy)", validateSceneStructure(view(unknownOp)));

  let truncationVerdictsMatch = true;
  for (let len = 0; len < bytes.length; len += 1) {
    const ok = validateSceneStructure(view(bytes.subarray(0, len)));
    const expected = boundaries.has(len);
    if (ok !== expected) truncationVerdictsMatch = false;
  }
  check("truncation at every byte boundary is detected exactly", truncationVerdictsMatch);
}

// ---- transactional renderPanel ---------------------------------------------

{
  const { bytes } = buildScene();
  const backs = [];
  const mod = new InstrumentModule(fakeExports({ sceneBytes: bytes, generation: () => 7 }), {
    createCanvas: () => {
      const c = recordingCanvas();
      backs.push(c);
      return c;
    },
  });
  const result = mod.renderPanel(PANEL.PFD);
  check("valid frame renders ok with the wasm generation", result.ok && result.generation === 7);
  check("frame painted to the back buffer, not a visible target", backs.length === 1);
  const backOps = backs[0].ctx.calls("fillRect");
  check(
    "back buffer cleared to full logical size before painting",
    backOps.length >= 2 && backOps[0].args.join(",") === `0,0,${LOGICAL_W},${LOGICAL_H}`,
  );
  const visible = new RecordingCtx();
  result.blit(visible, 960, 720);
  check(
    "blit covers the whole visible target before the frame",
    visible.log[0].op === "setTransform" &&
      visible.calls("fillRect")[0].args.join(",") === "0,0,960,720" &&
      visible.calls("drawImage").length === 1,
  );
}

{
  let created = 0;
  const mod = new InstrumentModule(fakeExports({ status: REASON.SCENE_BUFFER_FULL }), {
    createCanvas: () => {
      created += 1;
      return recordingCanvas();
    },
  });
  const result = mod.renderPanel(PANEL.PFD);
  check(
    "wasm failure code passes through untouched",
    !result.ok && result.reason === REASON.SCENE_BUFFER_FULL,
  );
  check("no back buffer touched on wasm failure", created === 0);
}

{
  const mod = new InstrumentModule(
    fakeExports({
      status: () => {
        throw new Error("trap");
      },
    }),
    { createCanvas: recordingCanvas },
  );
  check("a trapping render call is RENDER_TRAP", mod.renderPanel(0).reason === REASON.RENDER_TRAP);
}

{
  // Truncated mid-command: framing validation must reject before any paint.
  const { bytes } = buildScene();
  let created = 0;
  const mod = new InstrumentModule(fakeExports({ sceneBytes: bytes.subarray(0, bytes.length - 1) }), {
    createCanvas: () => {
      created += 1;
      return recordingCanvas();
    },
  });
  const result = mod.renderPanel(0);
  check("malformed scene is SCENE_FRAMING", !result.ok && result.reason === REASON.SCENE_FRAMING);
  check("malformed scene never reaches a canvas", created === 0);
}

{
  // A backend paint fault (canvas op throws) is contained to the back
  // buffer; the caller gets a typed failure and no blit.
  const { bytes } = buildScene();
  const throwing = recordingCanvas();
  throwing.ctx.fillRect = () => {
    throw new Error("canvas dead");
  };
  const mod = new InstrumentModule(fakeExports({ sceneBytes: bytes }), {
    createCanvas: () => throwing,
  });
  const result = mod.renderPanel(0);
  check("backend paint fault is PAINT_FAILED", !result.ok && result.reason === REASON.PAINT_FAILED);
}

// ---- failure latch, recovery, liveness (injected clock) ---------------------

{
  const health = new PanelHealth({ livenessDeadlineMs: 1000, recoveryFrames: 3 }, 0);
  check("healthy start shows no failure", health.display().showFailure === false);

  health.reportFailure(10, REASON.SCENE_FRAMING);
  check("failure latches with its reason", health.display().reason === REASON.SCENE_FRAMING);

  let d = health.reportSuccess(20, 1);
  check("one good frame cannot clear a latched failure", d.showFailure === true);
  d = health.reportSuccess(30, 2);
  check("two good frames still latched (recovery needs 3)", d.showFailure === true);
  d = health.reportSuccess(40, 3);
  check("sustained good frames clear the latch", d.showFailure === false);
  check("recovery counted", health.snapshot().counters.recoveries === 1);

  health.reportFailure(50, REASON.PAINT_FAILED);
  health.reportSuccess(60, 4);
  health.reportFailure(70, REASON.PAINT_FAILED);
  d = health.reportSuccess(80, 5);
  check("a failure during recovery resets the streak", d.showFailure === true);
}

{
  const health = new PanelHealth({ livenessDeadlineMs: 1000, recoveryFrames: 3 }, 0);
  health.reportSuccess(0, 1);
  check("at the deadline the panel is still live", health.tick(1000).showFailure === false);
  const d = health.tick(1001);
  check("past the deadline the panel fails LIVENESS", d.showFailure && d.reason === REASON.LIVENESS);
  check("liveness trip counted", health.snapshot().counters.livenessTrips === 1);

  // Recovery from a liveness latch still demands a sustained run.
  health.reportSuccess(1002, 2);
  check("one frame after a stall does not clear", health.display().showFailure === true);
  health.reportSuccess(1003, 3);
  health.reportSuccess(1004, 4);
  check("sustained frames clear a liveness latch", health.display().showFailure === false);
}

{
  const health = new PanelHealth({ livenessDeadlineMs: 1000, recoveryFrames: 3 }, 0);
  health.reportSuccess(0, 5);
  health.reportSuccess(500, 5); // duplicate generation: no freshness credit
  check("duplicate generation counted", health.snapshot().counters.duplicates === 1);
  const d = health.tick(1001);
  check(
    "repeated generations cannot keep a stalled panel alive",
    d.showFailure && d.reason === REASON.LIVENESS,
  );
}

{
  const health = new PanelHealth({ livenessDeadlineMs: 1000, recoveryFrames: 3 }, 0);
  health.reportSuccess(0, 0xffffffff);
  health.reportSuccess(900, 0); // u32 wrap is an advance, not a duplicate
  check("generation wrap counts as advancement", health.tick(1500).showFailure === false);
}

{
  const health = new PanelHealth({ livenessDeadlineMs: 1000, recoveryFrames: 3 }, 0);
  health.reportFailure(10, REASON.RENDER_TRAP);
  health.reset(2000);
  check("explicit reset is the reinitialization transition", health.display().showFailure === false);
  check("reset restarts the liveness deadline", health.tick(2999).showFailure === false);
}

// ---- failure page ------------------------------------------------------------

{
  const ctx = new RecordingCtx();
  drawFailurePage(ctx, 480, 360, REASON.LIVENESS);
  check(
    "failure page covers all previous imagery first",
    ctx.log[0].op === "setTransform" &&
      ctx.calls("fillRect")[0].args.join(",") === "0,0,480,360",
  );
  const texts = ctx.calls("fillText").map((e) => e.args[0]);
  check(
    "failure page shows DISPLAY FAIL and the stable code",
    texts.includes("DISPLAY FAIL") && texts.includes(`D-${REASON.LIVENESS}`),
  );
}

// ---- the real WASM module, end to end ----------------------------------------

{
  const wasmUrl = new URL("./instruments.wasm", import.meta.url);
  let exports = null;
  try {
    const { instance } = await WebAssembly.instantiate(readFileSync(wasmUrl), {});
    exports = instance.exports;
  } catch (error) {
    check(`instruments.wasm loads (build it: scripts/build-web-instruments.sh) — ${error}`, false);
  }
  if (exports) {
    for (const name of ["render_status", "render_generation", "scene_len"]) {
      check(`wasm exports ${name}`, typeof exports[name] === "function");
    }
    check("wasm init succeeds", exports.init() === 1);

    // The zeroed state block is a version-0 decode failure — a typed
    // code, not a silent zero-length scene.
    check("zeroed state is STATE_BAD_VERSION", exports.render_status(0) === REASON.STATE_BAD_VERSION);
    check("failed attempt does not advance the generation", exports.render_generation(0) === 0);
    check("unknown panel is INVALID_PANEL", exports.render_status(99) === REASON.INVALID_PANEL);

    const mod = new InstrumentModule(exports, { createCanvas: recordingCanvas });
    mod.writeState({
      attitude: { quat: { w: 1, x: 0, y: 0, z: 0 }, rates: [0, 0, 0], ageMs: 16 },
      kinematics: { posNed: [0, 0, -100], velNed: [10, 0, 0], ageMs: 16 },
      air: null,
      nav: null,
      wind: null,
      selections: { headingBugRad: 0 },
      quality: 0,
      valid: { attitude: true, rates: true, position: true, velocity: true },
    });
    for (const panel of [PANEL.PFD, PANEL.HSI]) {
      const result = mod.renderPanel(panel);
      check(`panel ${panel} renders and validates end to end`, result.ok === true);
      check(`panel ${panel} generation advanced`, exports.render_generation(panel) === 1);
    }
    check("scene bytes are non-trivial", exports.scene_len() > 1);
    const sceneView = new DataView(
      exports.memory.buffer,
      exports.scene_ptr(),
      exports.scene_len(),
    );
    check("real scene passes framing validation", validateSceneStructure(sceneView));
  }
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall instrument display checks passed");
