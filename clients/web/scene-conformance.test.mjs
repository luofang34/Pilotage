// Browser-side backend conformance against the shared golden corpus (REN-04).
//
// Run: node clients/web/scene-conformance.test.mjs
//
// The reference rasterizer (pilotage-instrument-raster) authors
// scene-conformance-corpus.json: for each case it pins the reference verdict,
// typed failure class, unknown-opcode count, layer-gate results, and a
// canonicalized decoded-command and Canvas draw trace. This test replays the
// same bytes through the browser backend — validateSceneStructure for framing,
// a wire decoder that mirrors the reference decoder, and interpretScene against
// a command-recording canvas — and asserts the browser's SEMANTIC outcome
// matches the pinned reference. Divergences surface as TYPED conformance
// failures (see DIVERGENCE), never tolerance comparisons.
//
// Capability asymmetry: the strong layer gate runs in wasm on wasm-generated
// scenes only, so it cannot be re-run here on arbitrary bytes. Gate verdict,
// layer-command counts, and reference render outcomes are therefore
// reference-only facts pinned in the golden and cross-checked on the Rust side;
// the browser side conforms at the framing, decode, and interpret levels. Where
// the two backends legitimately differ (the software rasterizer's non-finite,
// out-of-range, and vertex-budget fail-safes have no Canvas2D equivalent — the
// browser is SIM / NOT FOR FLIGHT), the golden documents both and each side
// checks its own.

import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import {
  COORD_LIMIT_PX,
  InstrumentModule,
  MAX_PATH_VERTICES,
  PANEL,
  STATE_ABI_SIZE,
  STATE_ABI_VERSION,
  interpretScene,
  validateSceneStructure,
} from "./instruments.js";
import { PanelHealth, REASON } from "./instrument-health.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// ---- wire helpers ----------------------------------------------------------

const view = (b) => new DataView(b.buffer, b.byteOffset, b.byteLength);

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(hex.substr(i * 2, 2), 16);
  return out;
}

function bytesToHex(bytes) {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

function f32bytes(v) {
  const b = new ArrayBuffer(4);
  new DataView(b).setFloat32(0, v, true);
  return [...new Uint8Array(b)];
}

function cmd(op, payload) {
  return [op, payload.length & 0xff, (payload.length >> 8) & 0xff, ...payload];
}

// Reconstructs a budget-boundary stream from its generator descriptor,
// byte-identical to the reference builder in corpus.rs.
function generate({ kind, layer, param }) {
  const b = [1];
  const open = () => {
    b.push(...cmd(0x50, [layer]));
    b.push(...cmd(0x01, []));
  };
  const close = () => {
    b.push(...cmd(0x02, []));
    b.push(...cmd(0x51, [layer]));
  };
  if (kind === "fill_bytes") {
    open();
    b.push(...cmd(0x7f, new Array(param - 18).fill(0)));
    close();
  } else if (kind === "repeat_unknown") {
    open();
    for (let i = 0; i < param; i += 1) b.push(...cmd(0x7f, []));
    close();
  } else if (kind === "nest_saves") {
    open();
    for (let i = 0; i < param; i += 1) b.push(...cmd(0x01, []));
    b.push(...cmd(0x23, [1, ...f32bytes(0), ...f32bytes(0), ...f32bytes(1), ...f32bytes(1)]));
    for (let i = 0; i < param; i += 1) b.push(...cmd(0x02, []));
    close();
  } else {
    throw new Error(`unknown generator kind ${kind}`);
  }
  return Uint8Array.from(b);
}

function entryBytes(entry) {
  return entry.generator ? generate(entry.generator) : hexToBytes(entry.bytesHex);
}

// ---- canonicalization (must match outcomes.rs) -----------------------------

function q(v) {
  if (typeof v === "boolean") return v ? "1" : "0";
  if (Number.isNaN(v)) return "nan";
  if (v === Infinity) return "inf";
  if (v === -Infinity) return "-inf";
  return String(Math.floor(v * 256));
}

const MODE_LETTERS = { 1: "F", 2: "S", 3: "FS" };

// A wire decoder mirroring pilotage-instrument-scene's SceneCmds/decode_payload:
// known-opcode payloads are shape-validated (a hard BadPayload), unknown opcodes
// are skipped by length. Returns { ok, error, trace } where trace matches the
// reference command trace exactly.
function decodeScene(bytes) {
  const v = view(bytes);
  if (bytes.length < 1) return { ok: false, error: "Truncated", trace: null };
  if (v.getUint8(0) !== 1) return { ok: false, error: "BadVersion", trace: null };
  let at = 1;
  const trace = [];
  while (at < bytes.length) {
    if (at + 3 > bytes.length) return { ok: false, error: "Truncated", trace: null };
    const op = v.getUint8(at);
    const plen = v.getUint16(at + 1, true);
    const p = at + 3;
    if (p + plen > bytes.length) return { ok: false, error: "Truncated", trace: null };
    const token = decodePayload(v, bytes, op, p, plen);
    if (token === null) return { ok: false, error: "BadPayload", trace: null };
    trace.push(token);
    at = p + plen;
  }
  return { ok: true, error: null, trace };
}

function decodePayload(v, bytes, op, p, plen) {
  const f = (i) => v.getFloat32(p + i * 4, true);
  const pts = (base, head) => {
    if ((plen - head) % 8 !== 0) return null;
    const parts = [];
    for (let i = 0; i < (plen - head) / 8; i += 1) {
      parts.push(q(v.getFloat32(base + i * 8, true)), q(v.getFloat32(base + i * 8 + 4, true)));
    }
    return parts.join(",");
  };
  switch (op) {
    case 0x01: return "01";
    case 0x02: return "02";
    case 0x03: return plen >= 8 ? `03:${q(f(0))},${q(f(1))}` : null;
    case 0x04: return plen >= 4 ? `04:${q(f(0))}` : null;
    case 0x10: return plen >= 4 ? `10:${color(v, p)}` : null;
    case 0x11: return plen >= 8 ? `11:${color(v, p)},${q(v.getFloat32(p + 4, true))}` : null;
    case 0x20: return plen >= 16 ? `20:${q(f(0))},${q(f(1))},${q(f(2))},${q(f(3))}` : null;
    case 0x21: { const s = pts(p, 0); return s === null ? null : `21:${s}`; }
    case 0x22: return polygonToken(v, p, plen, pts);
    case 0x23: return rectLike(v, p, plen, "23", 17);
    case 0x24: return rectLike(v, p, plen, "24", 13);
    case 0x25: return plen >= 20 ? `25:${q(f(0))},${q(f(1))},${q(f(2))},${q(f(3))},${q(f(4))}` : null;
    case 0x30: return textToken(v, bytes, p, plen);
    case 0x40: return plen >= 16 ? `40:${q(f(0))},${q(f(1))},${q(f(2))},${q(f(3))}` : null;
    case 0x50: return plen === 1 && v.getUint8(p) <= 5 ? `50:${v.getUint8(p)}` : null;
    case 0x51: return plen === 1 && v.getUint8(p) <= 5 ? `51:${v.getUint8(p)}` : null;
    default: return `unknown:${op}`;
  }
}

function color(v, p) {
  return `${v.getUint8(p)},${v.getUint8(p + 1)},${v.getUint8(p + 2)},${v.getUint8(p + 3)}`;
}

function polygonToken(v, p, plen, pts) {
  if (plen < 1) return null;
  const letters = MODE_LETTERS[v.getUint8(p) & 3];
  if (!letters) return null;
  const s = pts(p + 1, 1);
  return s === null ? null : `22:${letters}:${s}`;
}

function rectLike(v, p, plen, op, min) {
  if (plen < min) return null;
  const letters = MODE_LETTERS[v.getUint8(p) & 3];
  if (!letters) return null;
  const g = (i) => v.getFloat32(p + 1 + i * 4, true);
  if (op === "24") return `24:${letters},${q(g(0))},${q(g(1))},${q(g(2))}`;
  return `23:${letters},${q(g(0))},${q(g(1))},${q(g(2))},${q(g(3))}`;
}

function textToken(v, bytes, p, plen) {
  if (plen < 13) return null;
  const anchor = v.getUint8(p + 4);
  if ((anchor & 3) === 3) return null;
  const raw = bytes.subarray(p + 13, p + plen);
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(raw);
  } catch {
    return null;
  }
  const size = v.getFloat32(p, true);
  const x = v.getFloat32(p + 5, true);
  const y = v.getFloat32(p + 9, true);
  return `30:${q(size)},${anchor},${q(x)},${q(y)},${bytesToHex(raw)}`;
}

// ---- command-recording canvas ----------------------------------------------

class RecordingCtx {
  constructor() {
    this.log = [];
    const methods = [
      "save", "restore", "translate", "rotate", "setTransform", "beginPath", "moveTo",
      "lineTo", "closePath", "stroke", "fill", "fillRect", "strokeRect", "arc", "rect",
      "clip", "fillText", "drawImage",
    ];
    for (const m of methods) this[m] = (...args) => this.log.push({ op: m, args });
  }
  calls(op) {
    return this.log.filter((e) => e.op === op);
  }
}

function recordingCanvas(width = 0, height = 0) {
  const ctx = new RecordingCtx();
  return { width, height, getContext: () => ctx, ctx };
}

const NO_ARG_OPS = new Set(["save", "restore", "beginPath", "closePath", "stroke", "fill", "clip"]);

function canvasTrace(log) {
  return log.map((e) => (NO_ARG_OPS.has(e.op) ? e.op : `${e.op}:${e.args.map(q).join(",")}`));
}

// A minimal verified-shape atlas covering the digit the text corpus uses.
function fakeAtlas() {
  return {
    version: 1,
    cellW: 5,
    cellH: 7,
    advance: 6,
    baseline: 7,
    map: new Map([
      ["1".codePointAt(0), { advance: 6, rows: [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110] }],
    ]),
  };
}

// ---- typed conformance taxonomy --------------------------------------------

function DIVERGENCE(kind, entry, detail) {
  return { kind, entry, detail };
}

function traceDivergence(kind, name, expected, actual) {
  if (expected.length !== actual.length) {
    return DIVERGENCE(`${kind}LengthMismatch`, name, `expected ${expected.length}, got ${actual.length}`);
  }
  for (let i = 0; i < expected.length; i += 1) {
    if (expected[i] !== actual[i]) {
      return DIVERGENCE(`${kind}Divergence`, name, `index ${i}: expected ${expected[i]}, got ${actual[i]}`);
    }
  }
  return null;
}

function conform(entry, bytes) {
  const out = [];
  const v = view(bytes);
  if (validateSceneStructure(v) !== entry.framingValid) {
    out.push(DIVERGENCE("FramingMismatch", entry.name, `reference ${entry.framingValid}`));
  }
  const decoded = decodeScene(bytes);
  if (decoded.ok !== entry.decode.ok) {
    out.push(DIVERGENCE("DecodeVerdictMismatch", entry.name, `reference ok=${entry.decode.ok}, got ${decoded.ok}`));
  } else if (!decoded.ok && decoded.error !== entry.decode.error) {
    out.push(DIVERGENCE("DecodeClassMismatch", entry.name, `reference ${entry.decode.error}, got ${decoded.error}`));
  }
  if (entry.commandTrace) {
    const d = traceDivergence("CommandTrace", entry.name, entry.commandTrace, decoded.trace ?? []);
    if (d) out.push(d);
  }
  if (entry.canvasMethods || entry.interpreterRejects) conformCanvas(entry, v, out);
  return out;
}

// When the golden predicts a raw-argument rejection (interpreterRejects), the
// interpreter MUST throw before completing the scene — a clean pass means the
// browser-side guard is missing and malformed geometry would reach Canvas.
function conformCanvas(entry, v, out) {
  const ctx = new RecordingCtx();
  let unknown;
  try {
    unknown = interpretScene(v, ctx, null);
  } catch (error) {
    if (entry.interpreterRejects) return;
    out.push(DIVERGENCE("InterpretThrew", entry.name, String(error)));
    return;
  }
  if (entry.interpreterRejects) {
    out.push(DIVERGENCE("GuardMissing", entry.name, `expected ${entry.interpreterRejects} rejection, scene painted`));
    return;
  }
  if (unknown !== entry.gate.unknownOpcodes) {
    out.push(DIVERGENCE("UnknownOpcodeCountMismatch", entry.name, `reference ${entry.gate.unknownOpcodes}, got ${unknown}`));
  }
  const d = traceDivergence("Canvas", entry.name, entry.canvasMethods, canvasTrace(ctx.log));
  if (d) out.push(d);
}

// ---- load and pin the golden -----------------------------------------------

const golden = JSON.parse(
  readFileSync(new URL("./scene-conformance-corpus.json", import.meta.url), "utf8"),
);
check("golden schema version is understood", golden.schemaVersion === 2);
check("browser backend is declared SIM / NOT FOR FLIGHT", /SIM \/ NOT FOR FLIGHT/.test(golden.simOnly));
check(
  "budgets are the pinned resource bounds",
  golden.budgets.maxSceneBytes === 65536 &&
    golden.budgets.maxLayerCommands === 4096 &&
    golden.budgets.maxStackDepth === 32 &&
    golden.budgets.maxPolygonVertices === 512 &&
    golden.budgets.maxDimension === 4096 &&
    golden.budgets.maxPolygonVertices === MAX_PATH_VERTICES &&
    golden.budgets.coordLimitPx === COORD_LIMIT_PX,
);

const entries = golden.entries.map((e) => ({ entry: e, bytes: entryBytes(e) }));

// Corpus hash drift guard: both backends recompute it from the reconstructed
// bytes, so a Rust/JS generator divergence would surface here.
{
  const hash = createHash("sha256");
  for (const { bytes } of entries) hash.update(bytes);
  check("corpus hash matches the golden (no accidental drift)", hash.digest("hex") === golden.corpusSha256);
}

// ---- per-entry semantic conformance ----------------------------------------

{
  const divergences = [];
  for (const { entry, bytes } of entries) {
    if (entry.category === "truncation-sweep") continue;
    divergences.push(...conform(entry, bytes));
  }
  for (const d of divergences) console.error(`  DIVERGENCE ${d.kind} [${d.entry}]: ${d.detail}`);
  check(`every corpus case conforms (${entries.length} cases, 0 typed divergences)`, divergences.length === 0);
}

// ---- truncation sweep ------------------------------------------------------

{
  const sweep = golden.entries.find((e) => e.category === "truncation-sweep");
  const bytes = entryBytes(sweep);
  const boundaries = new Set(sweep.framingBoundaries);
  let matched = true;
  for (let len = 0; len <= bytes.length; len += 1) {
    const ok = validateSceneStructure(view(bytes.subarray(0, len)));
    if (ok !== boundaries.has(len)) matched = false;
  }
  check("truncation at every prefix length matches the reference framing boundaries", matched);
}

// ---- text: both backends fail an unrenderable run, neither substitutes ------

{
  const covered = entryBytes(golden.entries.find((e) => e.name === "text-covered"));
  const ctx = new RecordingCtx();
  const unknown = interpretScene(view(covered), ctx, fakeAtlas());
  check("covered text paints from the atlas as quads", ctx.calls("fillRect").length > 0 && unknown === 0);
  check("covered text never falls back to fillText", ctx.calls("fillText").length === 0);

  const uncovered = entryBytes(golden.entries.find((e) => e.name === "text-uncovered"));
  let threwUncovered = false;
  try {
    interpretScene(view(uncovered), new RecordingCtx(), fakeAtlas());
  } catch {
    threwUncovered = true;
  }
  check("uncovered character fails the run, never substitutes (agrees with reference Glyph error)", threwUncovered);

  let threwNoAtlas = false;
  try {
    interpretScene(view(covered), new RecordingCtx(), null);
  } catch {
    threwNoAtlas = true;
  }
  check("text without a verified atlas throws instead of falling back", threwNoAtlas);
}

// ---- fault injection through the transactional pipeline ---------------------

function packRenderResult(status, sceneLen, generation) {
  return (BigInt(generation >>> 0) << 32n) | (BigInt(sceneLen & 0xffffff) << 8n) | BigInt(status & 0xff);
}

function fakeRuntime({ status = 0, sceneBytes = new Uint8Array([1]), generation = () => 1 } = {}) {
  const buffer = new ArrayBuffer(65536);
  new Uint8Array(buffer).set(sceneBytes, 1024);
  const resolve = (fn, panel) => (typeof fn === "function" ? fn(panel) : fn);
  return {
    free() {}, init: () => 1, abi_version: () => STATE_ABI_VERSION,
    state_ptr: () => 256, state_len: () => STATE_ABI_SIZE, scene_ptr: () => 1024,
    set_v_speeds: () => 0, glyph_manifest: () => new Uint8Array(0), glyph_recorded_hash: () => new Uint8Array(0),
    render_result: (panel) => {
      const s = resolve(status, panel);
      return packRenderResult(s, s === 0 ? sceneBytes.length : 0, resolve(generation, panel));
    },
    memory: { buffer },
  };
}

{
  // A truncated corpus scene is framing-invalid: the pipeline rejects it as
  // SCENE_FRAMING before any paint, matching the reference framingValid=false.
  const truncated = entryBytes(golden.entries.find((e) => e.name === "truncated-tail"));
  let created = 0;
  const mod = new InstrumentModule(fakeRuntime({ sceneBytes: truncated }), {
    createCanvas: () => { created += 1; return recordingCanvas(); },
  });
  const visible = new RecordingCtx();
  const result = mod.renderPanel(PANEL.PFD, visible, 480, 360);
  check("a framing-invalid corpus scene is SCENE_FRAMING", !result.ok && result.reason === REASON.SCENE_FRAMING);
  check("a framing-invalid scene never reaches a canvas", created === 0 && visible.log.length === 0);
}

{
  // A backend paint fault (a Canvas op throws) is contained: typed
  // PAINT_FAILED, no visible blit.
  const scene = entryBytes(golden.entries.find((e) => e.name === "attitude-every-drawing-opcode"));
  const throwing = recordingCanvas();
  throwing.ctx.fillRect = () => { throw new Error("canvas dead"); };
  const mod = new InstrumentModule(fakeRuntime({ sceneBytes: scene }), { createCanvas: () => throwing });
  const result = mod.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
  check("a backend paint fault is PAINT_FAILED", !result.ok && result.reason === REASON.PAINT_FAILED);
}

{
  // A wasm-reported strong-gate failure passes through with its stable code and
  // latches display health; recovery demands sustained validated frames.
  const mod = new InstrumentModule(fakeRuntime({ status: REASON.SCENE_LAYER_CONTRACT }), {
    createCanvas: recordingCanvas,
  });
  const result = mod.renderPanel(PANEL.PFD, new RecordingCtx(), 480, 360);
  check("a wasm layer-contract failure passes through untouched", !result.ok && result.reason === REASON.SCENE_LAYER_CONTRACT);

  const health = new PanelHealth({ recoveryFrames: 3 }, 0);
  let d = health.reportFailure(1, result.reason);
  check("the layer-contract fault latches the panel with its reason", d.showFailure && d.reason === REASON.SCENE_LAYER_CONTRACT);
  health.reportSuccess(2, 1);
  d = health.reportSuccess(3, 2);
  check("one good frame does not clear the latch", d.showFailure);
  d = health.reportSuccess(4, 3);
  check("sustained validated frames recover the panel", !d.showFailure && health.snapshot().counters.recoveries === 1);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall scene-conformance checks passed");
