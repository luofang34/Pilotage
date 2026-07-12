// Deterministic checks for the fail-visible display pipeline.
//
// Run: node clients/web/instruments.test.mjs
// (build the generated WASM resource first: scripts/build-web-instruments.sh)
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
  STATE_ABI_SIZE,
  STATE_ABI_SIZE_BY_VERSION,
  STATE_ABI_VERSION,
  decodeRenderResult,
  interpretScene,
  loadInstruments,
  validateSceneStructure,
} from "./instruments.js";
import {
  PanelHealth,
  REASON,
  createDomFaultPresenter,
  drawFailurePage,
  renderInstrumentSet,
  startDisplayLoop,
} from "./instrument-health.js";

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

function recordingCanvas(width = 0, height = 0) {
  const ctx = new RecordingCtx();
  return { width, height, getContext: () => ctx, ctx };
}

function deadCtx() {
  return new Proxy(
    {},
    {
      get() {
        return () => {
          throw new Error("visible canvas failed");
        };
      },
      set() {
        throw new Error("visible canvas failed");
      },
    },
  );
}

function recordingPresenter() {
  return {
    active: false,
    reason: null,
    shows: 0,
    hides: 0,
    show(reason) {
      this.active = true;
      this.reason = reason;
      this.shows += 1;
    },
    hide() {
      this.active = false;
      this.hides += 1;
    },
  };
}

function packRenderResult(status, sceneLen, generation) {
  return (
    (BigInt(generation >>> 0) << 32n) |
    (BigInt(sceneLen & 0xffffff) << 8n) |
    BigInt(status & 0xff)
  );
}

// A fake WASM export surface with programmable behavior.
function fakeExports({
  status = 0,
  sceneBytes = new Uint8Array([1]),
  generation = () => 1,
  overrides = {},
} = {}) {
  const buffer = new ArrayBuffer(65536);
  new Uint8Array(buffer).set(sceneBytes, 1024);
  const renderStatus = typeof status === "function" ? status : () => status;
  const renderGeneration =
    typeof generation === "function" ? generation : () => generation;
  const exports = {
    abi_version: () => STATE_ABI_VERSION,
    free: () => {},
    init: () => 1,
    state_ptr: () => 256,
    state_len: () => STATE_ABI_SIZE,
    scene_ptr: () => 1024,
    render_result: (panel) => {
      const resultStatus = renderStatus(panel);
      const resultLen = resultStatus === 0 ? sceneBytes.length : 0;
      return packRenderResult(resultStatus, resultLen, renderGeneration(panel));
    },
    set_v_speeds: () => 0,
    memory: { buffer },
  };
  return Object.assign(exports, overrides);
}

function injectedLoader(exports) {
  return {
    loadBindings: async () => ({}),
    initializeBindings: async () => ({ memory: exports.memory }),
    createRuntime: () => exports,
    queryAbiVersion: () => exports.abi_version(),
    createCanvas: recordingCanvas,
  };
}

async function loadFailureReason(exports) {
  try {
    await loadInstruments("injected.wasm", injectedLoader(exports));
    return null;
  } catch (error) {
    return error?.reason ?? null;
  }
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

// ---- layer markers are known vocabulary ------------------------------------

{
  const layered = Uint8Array.from([
    1,
    ...cmd(0x50, [0]),
    ...cmd(0x01, []),
    ...cmd(0x23, [1, ...f32(0), ...f32(0), ...f32(4), ...f32(4)]),
    ...cmd(0x02, []),
    ...cmd(0x51, [0]),
  ]);
  const ctx = new RecordingCtx();
  check(
    "layer markers paint as known no-ops, not unknown opcodes",
    interpretScene(view(layered), ctx) === 0,
  );
  check(
    "marker envelope still paints its commands",
    ctx.calls("fillRect").length === 1,
  );
  const trulyUnknown = Uint8Array.from([1, ...cmd(0x7f, [])]);
  check(
    "a genuinely unknown opcode still counts",
    interpretScene(view(trulyUnknown), new RecordingCtx()) === 1,
  );
}

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
  check(
    "state ABI versions have exact independent sizes",
    STATE_ABI_SIZE_BY_VERSION[1] === 120 && STATE_ABI_SIZE_BY_VERSION[2] === 128,
  );
  const wrappedResult = decodeRenderResult(
    BigInt.asIntN(64, packRenderResult(REASON.OK, 65535, 0xffffffff)),
  );
  check(
    "packed render metadata survives signed i64 and generation wrap",
    wrappedResult?.status === REASON.OK &&
      wrappedResult?.sceneLen === 65535 &&
      wrappedResult?.generation === 0xffffffff,
  );
  const fieldMax = decodeRenderResult(packRenderResult(REASON.OK, 0xffffff, 1));
  check(
    "scene length decodes at the 24-bit field maximum",
    fieldMax?.sceneLen === 0xffffff && fieldMax?.status === REASON.OK,
  );
  const capacity = decodeRenderResult(packRenderResult(REASON.OK, 65536, 1));
  check(
    "scene length decodes at the real buffer capacity (bit 16 set)",
    capacity?.sceneLen === 65536 && capacity?.generation === 1,
  );
  check(
    "load rejects a non-function export as ABI_MISMATCH",
    (await loadFailureReason(fakeExports({ overrides: { render_result: 7 } }))) ===
      REASON.ABI_MISMATCH,
  );
  check(
    "load rejects an invalid memory export as ABI_MISMATCH",
    (await loadFailureReason(fakeExports({ overrides: { memory: null } }))) ===
      REASON.ABI_MISMATCH,
  );
  check(
    "load rejects an invalid ABI version",
    (await loadFailureReason(fakeExports({ overrides: { abi_version: () => 99 } }))) ===
      REASON.ABI_MISMATCH,
  );
  check(
    "ABI query trap has a stable reason",
    (await loadFailureReason(
      fakeExports({
        overrides: {
          abi_version: () => {
            throw new Error("ABI trap");
          },
        },
      }),
    )) === REASON.ABI_MISMATCH,
  );
  check(
    "init trap has a stable reason",
    (await loadFailureReason(
      fakeExports({
        overrides: {
          init: () => {
            throw new Error("init trap");
          },
        },
      }),
    )) === REASON.INIT_FAILED,
  );
  check(
    "init rejection has a stable reason",
    (await loadFailureReason(fakeExports({ overrides: { init: () => 0 } }))) ===
      REASON.INIT_FAILED,
  );
  check(
    "state length trap is ABI_MISMATCH",
    (await loadFailureReason(
      fakeExports({
        overrides: {
          state_len: () => {
            throw new Error("state length trap");
          },
        },
      }),
    )) === REASON.ABI_MISMATCH,
  );
  check(
    `ABI v${STATE_ABI_VERSION} requires exactly ${STATE_ABI_SIZE} state bytes`,
    (await loadFailureReason(
      fakeExports({ overrides: { state_len: () => (STATE_ABI_SIZE === 120 ? 128 : 120) } }),
    )) ===
      REASON.ABI_MISMATCH,
  );
  check(
    "buffer pointer trap is ABI_MISMATCH",
    (await loadFailureReason(
      fakeExports({
        overrides: {
          scene_ptr: () => {
            throw new Error("scene pointer trap");
          },
        },
      }),
    )) === REASON.ABI_MISMATCH,
  );
  check(
    "out-of-bounds state buffer is ABI_MISMATCH",
    (await loadFailureReason(
      fakeExports({ overrides: { state_ptr: () => 65536 - STATE_ABI_SIZE + 1 } }),
    )) === REASON.ABI_MISMATCH,
  );
  check(
    "zero state pointer after init is ABI_MISMATCH",
    (await loadFailureReason(fakeExports({ overrides: { state_ptr: () => 0 } }))) ===
      REASON.ABI_MISMATCH,
  );
  check(
    "zero scene pointer after init is ABI_MISMATCH",
    (await loadFailureReason(fakeExports({ overrides: { scene_ptr: () => 0 } }))) ===
      REASON.ABI_MISMATCH,
  );
  check(
    `load accepts the exact ABI v${STATE_ABI_VERSION} export surface`,
    (await loadInstruments("injected.wasm", injectedLoader(fakeExports()))) instanceof
      InstrumentModule,
  );
  let rejectedReleases = 0;
  const rejectedRuntime = fakeExports({
    overrides: {
      free: () => {
        rejectedReleases += 1;
      },
      state_len: () => STATE_ABI_SIZE - 1,
    },
  });
  await loadFailureReason(rejectedRuntime);
  check("rejected resource is released exactly once", rejectedReleases === 1);
  let fetchReason = null;
  try {
    await loadInstruments("missing.wasm", {
      ...injectedLoader(fakeExports()),
      initializeBindings: async () => {
        throw new Error("wasm load failed");
      },
    });
  } catch (error) {
    fetchReason = error?.reason ?? null;
  }
  check("generated-binding WASM failure is typed WASM_LOAD", fetchReason === REASON.WASM_LOAD);
}

{
  let releases = 0;
  const mod = new InstrumentModule(
    fakeExports({ overrides: { free: () => { releases += 1; } } }),
    { createCanvas: recordingCanvas },
  );
  mod.dispose();
  mod.dispose();
  check("explicit resource disposal is idempotent", releases === 1);
}

{
  const memory = { buffer: new ArrayBuffer(65536) };
  const statePtr = 256;
  const stale = new InstrumentModule(fakeExports({ overrides: { memory } }), {
    createCanvas: recordingCanvas,
    memory,
    statePtr,
    scenePtr: 1024,
  });
  stale.dispose();
  const replacement = new InstrumentModule(fakeExports({ overrides: { memory } }), {
    createCanvas: recordingCanvas,
    memory,
    statePtr,
    scenePtr: 1024,
  });
  const replacementState = new Uint8Array(memory.buffer, statePtr, STATE_ABI_SIZE);
  const replacementWrite = replacement.writeState({ quality: 2 });
  const beforeStaleWrite = replacementState.slice();

  const write = stale.writeState({});
  const speeds = stale.setVSpeeds(40, 50, 80, 120, 160);
  const render = stale.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
  check(
    "disposed wrapper cannot write into a replacement resource allocation",
    replacementWrite.ok &&
      !write.ok &&
      write.reason === REASON.STATE_WRITE_FAILED &&
      replacementState.every((byte, index) => byte === beforeStaleWrite[index]),
  );
  check(
    "all disposed resource entry points fail closed",
    !speeds.ok &&
      speeds.reason === REASON.RENDER_TRAP &&
      !render.ok &&
      render.reason === REASON.RENDER_TRAP,
  );
  replacement.dispose();
}

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
  const visible = new RecordingCtx();
  const result = mod.renderPanel(PANEL.PFD, visible, 960, 720);
  check("valid frame renders ok with the wasm generation", result.ok && result.generation === 7);
  check("frame painted to the back buffer, not a visible target", backs.length === 1);
  const backOps = backs[0].ctx.calls("fillRect");
  check(
    "back buffer cleared to full logical size before painting",
    backOps.length >= 2 && backOps[0].args.join(",") === `0,0,${LOGICAL_W},${LOGICAL_H}`,
  );
  check(
    "visible commit covers the whole target before the frame",
    visible.log[0].op === "setTransform" &&
      visible.calls("fillRect")[0].args.join(",") === "0,0,960,720" &&
      visible.calls("drawImage").length === 1,
  );
}

{
  let created = 0;
  const mod = new InstrumentModule(fakeExports({ status: REASON.SCENE_COMMAND_LIMIT }), {
    createCanvas: () => {
      created += 1;
      return recordingCanvas();
    },
  });
  const visible = new RecordingCtx();
  const result = mod.renderPanel(PANEL.PFD, visible, 480, 360);
  check(
    "command-limit failure code passes through untouched",
    !result.ok && result.reason === REASON.SCENE_COMMAND_LIMIT,
  );
  check("command-limit failure touches no back buffer", created === 0);
  check("command-limit failure leaves visible target untouched", visible.log.length === 0);
}

{
  for (const reason of [
    REASON.SCENE_BUFFER_FULL,
    REASON.SCENE_LAYER_CONTRACT,
    REASON.SCENE_CRITICAL_LAYERS_MISSING,
  ]) {
    let created = 0;
    const mod = new InstrumentModule(fakeExports({ status: reason }), {
      createCanvas: () => {
        created += 1;
        return recordingCanvas();
      },
    });
    const visible = new RecordingCtx();
    const result = mod.renderPanel(PANEL.PFD, visible, 480, 360);
    check(
      `wasm failure code ${reason} passes through untouched`,
      !result.ok && result.reason === reason,
    );
    check(`failure ${reason} touches no back buffer`, created === 0);
    check(`failure ${reason} leaves visible target untouched`, visible.log.length === 0);
  }
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
  check(
    "a trapping render call is RENDER_TRAP",
    mod.renderPanel(0, new RecordingCtx(), 480, 360).reason === REASON.RENDER_TRAP,
  );
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
  const result = mod.renderPanel(0, new RecordingCtx(), 480, 360);
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
  const result = mod.renderPanel(0, new RecordingCtx(), 480, 360);
  check("backend paint fault is PAINT_FAILED", !result.ok && result.reason === REASON.PAINT_FAILED);
}

{
  const { bytes } = buildScene();
  for (const [name, renderResult] of [
    [
      "render result trap",
      () => {
        throw new Error("render result trap");
      },
    ],
    ["non-BigInt render result", () => 0],
    [
      "failure carrying a scene length",
      () => packRenderResult(REASON.SCENE_BUFFER_FULL, 1, 0),
    ],
  ]) {
    const runtime = fakeExports({ sceneBytes: bytes });
    const mod = new InstrumentModule(runtime, {
      createCanvas: recordingCanvas,
    });
    runtime.render_result = renderResult;
    const result = mod.renderPanel(0, new RecordingCtx(), 480, 360);
    check(`${name} is a typed RENDER_TRAP`, !result.ok && result.reason === REASON.RENDER_TRAP);
  }
}

{
  const mod = new InstrumentModule(
    fakeExports({
      overrides: {
        memory: {
          get buffer() {
            throw new Error("runtime memory trap");
          },
        },
      },
    }),
    { createCanvas: recordingCanvas },
  );
  const result = mod.writeState({});
  check(
    "runtime state write trap is typed",
    !result.ok && result.reason === REASON.STATE_WRITE_FAILED,
  );
}

{
  const unavailable = new InstrumentModule(
    fakeExports({ overrides: { set_v_speeds: () => REASON.NOT_INITIALIZED } }),
    { createCanvas: recordingCanvas },
  );
  const failed = unavailable.setVSpeeds(40, 50, 80, 120, 160);
  check(
    "V-speed configuration returns its typed WASM failure",
    !failed.ok && failed.reason === REASON.NOT_INITIALIZED,
  );

  const trapping = new InstrumentModule(
    fakeExports({
      overrides: {
        set_v_speeds: () => {
          throw new Error("V-speed trap");
        },
      },
    }),
    { createCanvas: recordingCanvas },
  );
  check(
    "V-speed configuration trap is contained",
    trapping.setVSpeeds(40, 50, 80, 120, 160).reason === REASON.RENDER_TRAP,
  );
}

{
  const { bytes } = buildScene();
  let generation = 0;
  const mod = new InstrumentModule(
    fakeExports({
      sceneBytes: bytes,
      generation: () => {
        generation += 1;
        return generation;
      },
    }),
    { createCanvas: recordingCanvas },
  );
  const canvas = recordingCanvas(480, 360);
  const presenter = recordingPresenter();
  const health = { [PANEL.PFD]: new PanelHealth({}, 0) };
  const target = [PANEL.PFD, canvas.ctx, canvas, presenter];
  renderInstrumentSet(mod, health, [target], {}, 1);
  const hadFlightImage = canvas.ctx.calls("drawImage").length === 1;
  canvas.ctx.setTransform = () => {
    throw new Error("visible backend lost");
  };
  const [failure] = renderInstrumentSet(mod, health, [target], {}, 2);
  check(
    "a pre-existing flight image is covered when the visible backend fails",
    hadFlightImage &&
      failure.reason === REASON.PAINT_FAILED &&
      failure.covered &&
      presenter.active,
  );
}

{
  const { bytes } = buildScene();
  const mod = new InstrumentModule(
    fakeExports({ sceneBytes: bytes, generation: (panel) => panel + 1 }),
    { createCanvas: recordingCanvas },
  );
  const deadCanvas = { width: 480, height: 360 };
  const liveCanvas = recordingCanvas(480, 360);
  liveCanvas.ctx.log.push({ op: "existingFlightImage", args: [] });
  const deadPresenter = recordingPresenter();
  const livePresenter = recordingPresenter();
  const health = {
    [PANEL.PFD]: new PanelHealth({}, 0),
    [PANEL.HSI]: new PanelHealth({}, 0),
  };
  const outcomes = renderInstrumentSet(
    mod,
    health,
    [
      [PANEL.PFD, deadCtx(), deadCanvas, deadPresenter],
      [PANEL.HSI, liveCanvas.ctx, liveCanvas, livePresenter],
    ],
    {},
    10,
  );
  check(
    "dead visible Canvas activates the independent fault surface",
    outcomes[0].reason === REASON.PAINT_FAILED && outcomes[0].covered && deadPresenter.active,
  );
  check(
    "visible commit failure cannot receive health success",
    health[PANEL.PFD].snapshot().latched && health[PANEL.PFD].snapshot().lastGeneration === null,
  );
  check(
    "one panel Canvas failure does not block its peer",
    outcomes[1].ok &&
      liveCanvas.ctx.calls("drawImage").length === 1 &&
      health[PANEL.HSI].display().showFailure === false,
  );
}

{
  let renderCalls = 0;
  const brokenWriter = {
    writeState() {
      throw new Error("shared state write failed");
    },
    renderPanel() {
      renderCalls += 1;
      return { ok: true, generation: 1 };
    },
  };
  const canvases = [recordingCanvas(480, 360), recordingCanvas(480, 360)];
  const presenters = [recordingPresenter(), recordingPresenter()];
  const health = {
    [PANEL.PFD]: new PanelHealth({}, 0),
    [PANEL.HSI]: new PanelHealth({}, 0),
  };
  const outcomes = renderInstrumentSet(
    brokenWriter,
    health,
    [
      [PANEL.PFD, canvases[0].ctx, canvases[0], presenters[0]],
      [PANEL.HSI, canvases[1].ctx, canvases[1], presenters[1]],
    ],
    {},
    10,
  );
  check(
    "shared write fault latches and covers both panels",
    outcomes.every((outcome) => outcome.reason === REASON.STATE_WRITE_FAILED && outcome.covered) &&
      presenters.every((presenter) => presenter.active),
  );
  check("shared write fault suppresses panel rendering", renderCalls === 0);
}

{
  let generation = 0;
  const mod = new InstrumentModule(
    fakeExports({
      generation: () => {
        generation += 1;
        return generation;
      },
    }),
    { createCanvas: recordingCanvas },
  );
  const canvas = recordingCanvas(480, 360);
  const presenter = recordingPresenter();
  const health = { [PANEL.PFD]: new PanelHealth({ recoveryFrames: 2 }, 0) };
  health[PANEL.PFD].reportFailure(1, REASON.PAINT_FAILED);
  const target = [PANEL.PFD, canvas.ctx, canvas, presenter];
  renderInstrumentSet(mod, health, [target], {}, 2);
  check("one committed frame leaves the recovery overlay latched", presenter.active);
  renderInstrumentSet(mod, health, [target], {}, 3);
  check(
    "the independent overlay clears after sustained committed frames",
    !presenter.active && health[PANEL.PFD].display().showFailure === false,
  );
}

{
  const queued = [];
  let reports = 0;
  startDisplayLoop(
    (callback) => queued.push(callback),
    () => {
      throw new Error("render loop fault");
    },
    () => {
      reports += 1;
      throw new Error("fault presenter fault");
    },
  );
  const firstFrame = queued.shift();
  firstFrame(10);
  check(
    "rAF is rescheduled after render and fault-reporting exceptions",
    reports === 1 && queued.length === 1,
  );
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
  const attributes = new Map();
  const element = {
    style: {},
    setAttribute(name, value) {
      attributes.set(name, value);
    },
    textContent: "",
  };
  let inserted = null;
  const canvas = {
    nextSibling: null,
    parentElement: {
      insertBefore(value) {
        inserted = value;
      },
    },
  };
  const originalDocument = globalThis.document;
  globalThis.document = { createElement: () => element };
  try {
    const presenter = createDomFaultPresenter(canvas);
    presenter.show(REASON.PAINT_FAILED);
    check(
      "DOM fault presenter is an independent full-cover surface",
      inserted === element &&
        element.style.position === "absolute" &&
        element.style.inset === "0" &&
        element.style.display === "grid" &&
        element.textContent.includes("DISPLAY FAIL") &&
        element.textContent.includes("SIM / NOT FOR FLIGHT") &&
        attributes.get("aria-hidden") === "false",
    );
    presenter.hide();
    check(
      "DOM fault presenter hides only on explicit recovery",
      element.style.display === "none" && attributes.get("aria-hidden") === "true",
    );
  } finally {
    if (originalDocument === undefined) delete globalThis.document;
    else globalThis.document = originalDocument;
  }
}

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

// ---- VAL-01: fail-safe write defaults mirror abi.rs ---------------------------

{
  const exportsFake = fakeExports({});
  const mod = new InstrumentModule(exportsFake, { createCanvas: recordingCanvas });
  const minimal = mod.writeState({ selections: { headingBugRad: 0 } });
  check("minimal state write succeeds", minimal.ok === true);
  const view = new DataView(exportsFake.memory.buffer, exportsFake.state_ptr(), STATE_ABI_SIZE);
  check(
    "undeclared quality writes the unknown code (abi.rs parity), never Good",
    view.getUint8(84) === 255,
  );
  check("undeclared validity writes no flags (nothing declared valid)", view.getUint8(85) === 0);

  const declared = mod.writeState({
    selections: { headingBugRad: 0 },
    quality: 1,
    valid: { attitude: true, rates: true, position: true, velocity: true },
  });
  check("declared trust write succeeds", declared.ok === true);
  check(
    "declared quality and flags encode exactly",
    view.getUint8(84) === 1 && view.getUint8(85) === 0b1111,
  );

  const partial = mod.writeState({
    selections: { headingBugRad: 0 },
    quality: 0,
    valid: { attitude: true, velocity: true },
  });
  check("partial validity write succeeds", partial.ok === true);
  check(
    "undeclared flags stay unset within a partial declaration",
    view.getUint8(85) === 0b1001,
  );
}

// ---- the real WASM module, end to end ----------------------------------------

{
  const wasmUrl = new URL("./instrument-runtime_bg.wasm", import.meta.url);
  let mod = null;
  let capturedWasm = null;
  try {
    const bindings = await import("./instrument-runtime.js");
    mod = await loadInstruments(readFileSync(wasmUrl), {
      createCanvas: recordingCanvas,
      loadBindings: async () => bindings,
      initializeBindings: async (args) => {
        capturedWasm = await bindings.default(args);
        return capturedWasm;
      },
    });
  } catch (error) {
    check(`instrument runtime loads (build it: scripts/build-web-instruments.sh) — ${error}`, false);
  }
  if (mod) {
    check("real wasm passes load, ABI, init, and exact-size validation", mod instanceof InstrumentModule);
    check("raw WASM resource is encapsulated by the validated module", !("runtime" in mod));

    // The zeroed state block is a version-0 decode failure — a typed
    // code, not a silent zero-length scene.
    const failed = mod.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
    check("zeroed state is STATE_BAD_VERSION", failed.reason === REASON.STATE_BAD_VERSION);
    const invalidPanel = mod.renderPanel(99, new RecordingCtx(), 480, 360);
    check("unknown panel is INVALID_PANEL", invalidPanel.reason === REASON.INVALID_PANEL);

    const writeResult = mod.writeState({
      attitude: { quat: { w: 1, x: 0, y: 0, z: 0 }, rates: [0, 0, 0], ageMs: 16 },
      kinematics: { posNed: [0, 0, -100], velNed: [10, 0, 0], ageMs: 16 },
      air: null,
      nav: null,
      wind: null,
      selections: { headingBugRad: 0 },
      quality: 0,
      valid: { attitude: true, rates: true, position: true, velocity: true },
    });
    check("real wasm state write succeeds", writeResult.ok === true);
    let lastResult = null;
    for (const panel of [PANEL.PFD, PANEL.HSI]) {
      const result = mod.renderPanel(panel, new RecordingCtx(), 480, 360);
      check(`panel ${panel} renders and validates end to end`, result.ok === true);
      check(
        `panel ${panel} result carries its first generation after failed attempts`,
        result.generation === 1,
      );
      lastResult = result;
    }
    const lastSceneLen = lastResult?.sceneLen ?? 0;
    check("atomic result carries a non-trivial scene length", lastSceneLen > 1);
    check("real scene passed framing before visible commit", lastResult?.ok === true);
    check(
      "a real layered frame leaves the unknown-opcode diagnostic clean",
      mod.unknownOpcodes === 0,
    );
    // Linear-memory growth detaches every previously created view; the
    // module must re-derive views per frame, never cache one.
    capturedWasm.memory.grow(1);
    const grown = mod.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
    check(
      "memory growth between frames does not stale the pipeline",
      grown.ok === true && grown.generation === 2,
    );

    // Invalid aircraft data is not a display failure: the frame renders
    // (in-scene red-X/flags), the pipeline reports success, and nothing
    // covers the panel with the failure page.
    const invalidWrite = mod.writeState({
      attitude: { quat: { w: 1, x: 0, y: 0, z: 0 }, rates: [0, 0, 0], ageMs: 16 },
      kinematics: { posNed: [0, 0, -100], velNed: [10, 0, 0], ageMs: 16 },
      air: null,
      nav: null,
      wind: null,
      selections: { headingBugRad: 0 },
      quality: 0,
      valid: { attitude: false, rates: false, position: true, velocity: true },
    });
    check("invalid-data state write succeeds", invalidWrite.ok === true);
    const invalidData = mod.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
    check(
      "invalid aircraft data renders as a successful flagged frame, not DISPLAY FAIL",
      invalidData.ok === true && invalidData.generation === 3,
    );
    mod.dispose();
  }
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall instrument display checks passed");
