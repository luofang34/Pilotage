// Browser backend for the instrument runtime (ADR-0017).
//
// Loads the pilotage-instruments-web WASM module (built by
// scripts/build-web-instruments.sh), writes packed aircraft state into its
// linear memory (mirroring pilotage-instrument-state/src/abi.rs exactly),
// and interprets the returned scene-command bytes onto a Canvas2D.
//
// Rendering is transactional: a frame reaches the visible canvas
// only after the WASM reports success, the scene bytes pass structural
// validation, and the full frame painted onto an offscreen back buffer.
// Every failure returns a stable reason code; no path silently keeps the
// previous image.
//
// The interpreter is one backend of the versioned scene IR; unknown
// opcodes are skipped and counted, never fatal (ADR-0017).

import { InstrumentFault, REASON } from "./instrument-health.js";

// Panel map derived from the wasm registry enumeration at load time
// (ADR-0029): keys are the descriptor ids uppercased, values the panel
// indices. The script holds no panel list of its own; a pin test keeps
// the derived map equal to the shape consumers rely on.
export const PANEL = {};

// Per-panel design frames from the descriptors, filled at load; the
// fallback only guards a render attempted before any load completes.
const DESIGN = new Map();
const FALLBACK_DESIGN = Object.freeze({ width: 480, height: 360 });

function designOf(panel) {
  return DESIGN.get(panel) ?? FALLBACK_DESIGN;
}

import { STATE_ABI_VERSION, encodeState, maxFrameBytes } from "./state-abi.js";

const SCENE_FORMAT_VERSION = 1;

// The controlled glyph pack's recorded content hash (REN-02). The backend
// verifies the wasm-exported canonical bytes against BOTH the wasm's
// recorded hash and this pinned value before declaring ready, so a stale
// or corrupt asset — on either side — fails visibly instead of falling
// back to a system font.
export const EXPECTED_GLYPH_SHA256 =
  "281eef6229feee417c7090d8c8ea79489c017cd1c02fc7234876b2a64a532158";
const GLYPH_HEADER_LEN = 8;
const GLYPH_RECORD_LEN = 12;
const GLYPH_ROWS = 7;
export { STATE_ABI_VERSION } from "./state-abi.js";
// The v6 state frame is self-delimiting; the wasm side publishes a
// buffer capacity the writer stays within. The floor is the largest
// canonical frame the writer can produce, measured, not mirrored.
const STATE_MIN_CAPACITY = maxFrameBytes();
const MAX_WASM_RENDER_STATUS = 12;

// A resource missing any required method is incompatible and must fail as an
// ABI mismatch rather than as a TypeError mid-frame.
const REQUIRED_RUNTIME_METHODS = [
  "free",
  "init",
  "state_ptr",
  "state_capacity",
  "scene_ptr",
  "render_result",
  "set_v_speeds",
  "set_panel_config",
  "state_unknown_groups",
  "state_extended_groups",
  "step_alerts",
  "glyph_manifest",
  "glyph_recorded_hash",
];

// The pinned cross-shell scene digest (ADR-0033). The wasm build must
// reproduce exactly this value over its composed registry and the
// canonical corpus — pinned here as a JS literal (the glyph-hash
// pattern) so wasm-target divergence from the host-verified contract
// fails the suite, not just a Rust unit test.
export const EXPECTED_SCENE_DIGEST =
  "a809f1768d03f533b134e15ddf1a49779565f03fc829164fb6c94b357bdb1abc";

// Registry enumeration exported at module level by the bindings; the
// backend derives its panel map from these instead of mirroring one.
const REQUIRED_BINDING_FNS = [
  "panel_count",
  "panel_id",
  "panel_title",
  "panel_required_layers",
  "panel_required_groups",
  "panel_design_width",
  "panel_design_height",
  "panel_background_capability",
];

export async function loadInstruments(wasmUrl, options = {}) {
  let bindings;
  try {
    bindings = await (options.loadBindings ?? (() => import("./instrument-runtime.js")))();
  } catch (error) {
    throw new InstrumentFault(REASON.WASM_LOAD, `instrument binding load failed: ${error}`);
  }
  const initializeBindings = options.initializeBindings ?? bindings.default;
  const createRuntime = options.createRuntime ?? (() => new bindings.InstrumentRuntime());
  const queryAbiVersion = options.queryAbiVersion ?? bindings.abi_version;
  if (
    typeof initializeBindings !== "function" ||
    typeof createRuntime !== "function" ||
    typeof queryAbiVersion !== "function"
  ) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, "instrument binding surface is incomplete");
  }
  const enumeration = options.enumeration ?? bindings;
  for (const name of REQUIRED_BINDING_FNS) {
    if (typeof enumeration?.[name] !== "function") {
      throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument binding lacks ${name}`);
    }
  }
  let wasm;
  try {
    wasm = await initializeBindings({ module_or_path: wasmUrl });
  } catch (error) {
    throw new InstrumentFault(REASON.WASM_LOAD, `instrument wasm load failed: ${error}`);
  }
  const memoryBuffer = wasm?.memory?.buffer;
  const validMemory =
    memoryBuffer instanceof ArrayBuffer ||
    (typeof SharedArrayBuffer !== "undefined" && memoryBuffer instanceof SharedArrayBuffer);
  if (!validMemory) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, "instrument binding has invalid memory");
  }
  let runtime;
  try {
    runtime = createRuntime();
  } catch (error) {
    throw new InstrumentFault(REASON.INIT_FAILED, `instrument runtime construction failed: ${error}`);
  }
  for (const name of REQUIRED_RUNTIME_METHODS) {
    if (typeof runtime?.[name] !== "function") {
      releaseRuntime(runtime);
      throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument runtime has invalid method ${name}`);
    }
  }
  let abiVersion;
  try {
    abiVersion = queryAbiVersion();
  } catch (error) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument ABI query failed: ${error}`);
  }
  if (abiVersion !== STATE_ABI_VERSION) {
    releaseRuntime(runtime);
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument ABI mismatch: wasm=${abiVersion} js=${STATE_ABI_VERSION}`,
    );
  }
  let initialized;
  try {
    initialized = runtime.init();
  } catch (error) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.INIT_FAILED, `instrument wasm init trapped: ${error}`);
  }
  if (initialized !== 1) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.INIT_FAILED, `instrument wasm init returned ${initialized}`);
  }
  let stateCapacity;
  try {
    stateCapacity = runtime.state_capacity();
  } catch (error) {
    releaseRuntime(runtime);
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument state capacity query failed: ${error}`,
    );
  }
  if (!Number.isInteger(stateCapacity) || stateCapacity < STATE_MIN_CAPACITY) {
    releaseRuntime(runtime);
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument state capacity invalid: wasm=${stateCapacity}`,
    );
  }
  // Derive the panel map and design frames from the registry
  // enumeration; the shell holds no panel list of its own (ADR-0029).
  let panelCount;
  try {
    panelCount = enumeration.panel_count();
  } catch (error) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.ABI_MISMATCH, `panel enumeration failed: ${error}`);
  }
  if (!Number.isInteger(panelCount) || panelCount < 1) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.ABI_MISMATCH, `panel count invalid: ${panelCount}`);
  }
  // Build into locals and swap only after the whole enumeration
  // validates: a broken second load must not clobber the map a working
  // module's consumers are already using.
  const nextPanel = {};
  const nextDesign = new Map();
  const panels = [];
  for (let index = 0; index < panelCount; index += 1) {
    const id = enumeration.panel_id(index);
    const width = enumeration.panel_design_width(index);
    const height = enumeration.panel_design_height(index);
    if (typeof id !== "string" || id.length === 0 || !(width > 0) || !(height > 0)) {
      releaseRuntime(runtime);
      throw new InstrumentFault(REASON.ABI_MISMATCH, `panel descriptor ${index} invalid`);
    }
    nextPanel[id.toUpperCase()] = index;
    nextDesign.set(index, Object.freeze({ width, height }));
    panels.push(
      Object.freeze({
        index,
        id,
        title: enumeration.panel_title(index),
        requiredLayers: enumeration.panel_required_layers(index),
        requiredGroups: enumeration.panel_required_groups(index),
        width,
        height,
        background: enumeration.panel_background_capability(index),
      }),
    );
  }
  for (const key of Object.keys(PANEL)) delete PANEL[key];
  DESIGN.clear();
  Object.assign(PANEL, nextPanel);
  for (const [index, design] of nextDesign) DESIGN.set(index, design);
  let statePtr;
  let scenePtr;
  try {
    statePtr = runtime.state_ptr();
    scenePtr = runtime.scene_ptr();
  } catch (error) {
    releaseRuntime(runtime);
    throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument buffer query failed: ${error}`);
  }
  const activeMemoryBuffer = wasm.memory.buffer;
  const memoryLen = activeMemoryBuffer.byteLength;
  const stateFits =
    Number.isInteger(statePtr) && statePtr > 0 && statePtr + stateCapacity <= memoryLen;
  const sceneStartsInMemory =
    Number.isInteger(scenePtr) && scenePtr > 0 && scenePtr < memoryLen;
  if (!stateFits || !sceneStartsInMemory) {
    releaseRuntime(runtime);
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument buffer layout invalid: state=${statePtr} scene=${scenePtr} memory=${memoryLen}`,
    );
  }
  let glyphs;
  try {
    glyphs = options.glyphAtlas ?? (await loadVerifiedGlyphAtlas(runtime));
  } catch (error) {
    releaseRuntime(runtime);
    throw error;
  }
  const mod = new InstrumentModule(runtime, {
    ...options,
    memory: wasm.memory,
    statePtr,
    stateCapacity,
    scenePtr,
    glyphs,
  });
  // The registry composition, on the instance: consumers that must not
  // depend on load-order side effects read this instead of the module
  // globals.
  mod.panels = Object.freeze(panels);
  return mod;
}

/// Loads the wasm-exported glyph canonical bytes, verifies them against
/// both the wasm-recorded hash and the pinned EXPECTED_GLYPH_SHA256, and
/// parses the atlas. Any shortfall is a typed GLYPH_ASSET fault: the
/// backend never becomes ready over a missing, corrupt, or wrong-hash
/// asset, and never substitutes a system font.
async function loadVerifiedGlyphAtlas(runtime) {
  let canonical;
  let recorded;
  try {
    canonical = runtime.glyph_manifest();
    recorded = runtime.glyph_recorded_hash();
  } catch (error) {
    throw new InstrumentFault(REASON.GLYPH_ASSET, `glyph asset query failed: ${error}`);
  }
  if (!(canonical instanceof Uint8Array) || !(recorded instanceof Uint8Array)) {
    throw new InstrumentFault(REASON.GLYPH_ASSET, "glyph asset export shape invalid");
  }
  const recordedHex = [...recorded].map((b) => b.toString(16).padStart(2, "0")).join("");
  if (recordedHex !== EXPECTED_GLYPH_SHA256) {
    throw new InstrumentFault(
      REASON.GLYPH_ASSET,
      `glyph recorded hash ${recordedHex} does not match the pinned pack`,
    );
  }
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", canonical));
  const digestHex = [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
  if (digestHex !== EXPECTED_GLYPH_SHA256) {
    throw new InstrumentFault(
      REASON.GLYPH_ASSET,
      `glyph canonical bytes hash ${digestHex}, expected the pinned pack`,
    );
  }
  return parseGlyphAtlas(canonical);
}

/// Parses the canonical glyph serialization (verified by hash above):
/// 8-byte header (version u16 LE, cell w/h u8, advance u8, baseline u8,
/// count u16 LE) then 12 bytes per glyph (char u32 LE, advance u8, seven
/// row bitmaps, leftmost column = highest used bit).
export function parseGlyphAtlas(canonical) {
  if (canonical.length < GLYPH_HEADER_LEN) {
    throw new InstrumentFault(REASON.GLYPH_ASSET, "glyph canonical too short");
  }
  const view = new DataView(canonical.buffer, canonical.byteOffset, canonical.byteLength);
  const count = view.getUint16(6, true);
  if (canonical.length !== GLYPH_HEADER_LEN + count * GLYPH_RECORD_LEN) {
    throw new InstrumentFault(REASON.GLYPH_ASSET, "glyph canonical length mismatch");
  }
  const map = new Map();
  for (let i = 0; i < count; i += 1) {
    const at = GLYPH_HEADER_LEN + i * GLYPH_RECORD_LEN;
    const rows = [];
    for (let r = 0; r < GLYPH_ROWS; r += 1) rows.push(canonical[at + 5 + r]);
    map.set(view.getUint32(at, true), { advance: canonical[at + 4], rows });
  }
  return {
    version: view.getUint16(0, true),
    cellW: canonical[2],
    cellH: canonical[3],
    advance: canonical[4],
    baseline: canonical[5],
    map,
  };
}

function releaseRuntime(runtime) {
  try {
    runtime?.free?.();
  } catch {
    // The resource is already unusable; its initialization fault remains primary.
  }
}

// Structural validation of an encoded scene: version byte plus exact
// command framing ([opcode u8][payload_len u16 LE][payload]) covering the
// buffer with no trailing partial command. Runs BEFORE any painting so a
// malformed scene can never become partially visible. Unknown opcodes are
// a version-policy concern, not a structural one — they pass here and are
// counted by the interpreter.
export function validateSceneStructure(view) {
  if (view.byteLength < 1 || view.getUint8(0) !== SCENE_FORMAT_VERSION) return false;
  let at = 1;
  while (at + 3 <= view.byteLength) {
    at += 3 + view.getUint16(at + 1, true);
  }
  return at === view.byteLength;
}

export class InstrumentModule {
  #runtime;
  #memory;
  #statePtr;
  #stateCapacity;
  #scenePtr;
  #disposed;

  // options.createCanvas injects the back-buffer factory (tests pass a
  // recording double; the browser default is a plain offscreen element).
  #glyphs;

  constructor(runtime, { createCanvas, memory, statePtr, stateCapacity, scenePtr, glyphs } = {}) {
    this.#glyphs = glyphs ?? null;
    this.#runtime = runtime;
    this.#memory = memory ?? runtime.memory;
    this.#statePtr = statePtr ?? runtime.state_ptr();
    this.#stateCapacity = stateCapacity ?? runtime.state_capacity();
    this.#scenePtr = scenePtr ?? runtime.scene_ptr();
    this.#disposed = false;
    this.unknownOpcodes = 0;
    this.createCanvas =
      createCanvas ??
      ((w, h) => {
        const canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        return canvas;
      });
    this.back = new Map();
  }

  dispose() {
    if (this.#disposed) return;
    this.#disposed = true;
    const runtime = this.#runtime;
    this.#runtime = null;
    this.#memory = null;
    this.#statePtr = 0;
    this.#scenePtr = 0;
    this.back.clear();
    runtime.free();
  }

  setVSpeeds(vs0, vs, vfe, vno, vne) {
    if (this.#disposed) return { ok: false, reason: REASON.RENDER_TRAP };
    try {
      const status = this.#runtime.set_v_speeds(vs0, vs, vfe, vno, vne);
      if (!Number.isInteger(status) || status < 0 || status > MAX_WASM_RENDER_STATUS) {
        return { ok: false, reason: REASON.RENDER_TRAP };
      }
      return status === 0 ? { ok: true } : { ok: false, reason: status };
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
  }

  // Steps the alert manager once for this frame; every panel rendered
  // afterward consumes the one cached output, so the panels can never
  // disagree on the semantic alert state. pathHealthy carries the
  // independent display-watchdog verdict into the manager (false marks
  // the output untrusted without suppressing it).
  /// Wrapping count of decoded frames' group tags this build cannot
  /// place — the state-path sibling of unknownOpcodes.
  stateUnknownGroups() {
    if (this.#disposed) return 0;
    try {
      return this.#runtime.state_unknown_groups();
    } catch {
      return 0;
    }
  }

  /// Wrapping count of known groups whose payloads carried grown tails.
  stateExtendedGroups() {
    if (this.#disposed) return 0;
    try {
      return this.#runtime.state_extended_groups();
    } catch {
      return 0;
    }
  }

  stepAlerts(nowMs, pathHealthy) {
    if (this.#disposed) return { ok: false, reason: REASON.RENDER_TRAP };
    try {
      const packed = this.#runtime.step_alerts(
        BigInt(Math.max(0, Math.round(nowMs))),
        pathHealthy ? 1 : 0,
      );
      if (typeof packed !== "bigint") return { ok: false, reason: REASON.RENDER_TRAP };
      const status = Number(packed & 0xffn);
      if (status < 0 || status > MAX_WASM_RENDER_STATUS) {
        return { ok: false, reason: REASON.RENDER_TRAP };
      }
      if (status !== 0) return { ok: false, reason: status };
      return {
        ok: true,
        active: Number((packed >> 8n) & 0xffn),
        faulted: ((packed >> 16n) & 1n) === 1n,
        overflow: ((packed >> 17n) & 1n) === 1n,
      };
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
  }

  // state: the group-keyed object state-abi.js encodes; a group key that
  // is absent emits no tag, and the wasm side resolves that group
  // Missing by construction. Field-level fail-safe defaults (unknown
  // enums, undeclared trust) live in the shared writer.
  writeState(state) {
    if (this.#disposed) return { ok: false, reason: REASON.STATE_WRITE_FAILED };
    try {
      const view = new DataView(this.#memory.buffer, this.#statePtr, this.#stateCapacity);
      encodeState(view, state);
      return { ok: true };
    } catch {
      return { ok: false, reason: REASON.STATE_WRITE_FAILED };
    }
  }

  // Renders `panel` transactionally. Returns either
  //   { ok: true, generation }
  // after the validated frame covers the whole visible target, or
  //   { ok: false, reason }
  // with a stable REASON code. The caller must cover failures with the
  // backend-owned failure page, never leave the last visible frame.
  renderPanel(panel, ctx, width, height) {
    if (this.#disposed) return { ok: false, reason: REASON.RENDER_TRAP };
    // Shell layouts key their surfaces by descriptor id; indices exist
    // only after enumeration, so the id dialect is resolved here where
    // a loaded module is a given.
    if (typeof panel === "string") {
      panel = PANEL[panel.toUpperCase()];
    }
    if (!Number.isInteger(panel)) return { ok: false, reason: REASON.INVALID_PANEL };
    if (!Number.isFinite(width) || width <= 0 || !Number.isFinite(height) || height <= 0) {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    let result;
    try {
      result = decodeRenderResult(this.#runtime.render_result(panel));
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (result === null) return { ok: false, reason: REASON.RENDER_TRAP };
    const { status, sceneLen: len, generation } = result;
    if (!Number.isInteger(status) || status < 0 || status > MAX_WASM_RENDER_STATUS) {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (status !== 0) {
      return len === 0
        ? { ok: false, reason: status }
        : { ok: false, reason: REASON.RENDER_TRAP };
    }
    let view;
    try {
      view = new DataView(this.#memory.buffer, this.#scenePtr, len);
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    let structurallyValid;
    try {
      structurallyValid = validateSceneStructure(view);
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (!structurallyValid) return { ok: false, reason: REASON.SCENE_FRAMING };
    let back;
    try {
      const design = designOf(panel);
      back = this.back.get(panel);
      if (!back) {
        back = this.createCanvas(design.width, design.height);
        this.back.set(panel, back);
      }
      // Re-assigning width resets the back context wholesale (state
      // stack, transform, clip), so an unbalanced save/clip in one frame
      // cannot leak into the next.
      back.width = design.width;
      back.height = design.height;
      const bctx = back.getContext("2d");
      bctx.fillStyle = "#000";
      bctx.fillRect(0, 0, design.width, design.height);
      const unknownThisFrame = interpretScene(view, bctx, this.#glyphs);
      this.unknownOpcodes += unknownThisFrame;
      if (unknownThisFrame > 0) {
        // A skipped opcode means draw commands silently dropped out of
        // the frame — a lost clip or tape backdrop bleeds layers that
        // must never show through. This is a scene/interpreter
        // opcode-set mismatch; never present such a frame.
        return { ok: false, reason: REASON.UNKNOWN_OPCODE };
      }
    } catch {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    try {
      const design = designOf(panel);
      const scale = Math.min(width / design.width, height / design.height);
      const dw = design.width * scale;
      const dh = design.height * scale;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, width, height);
      ctx.drawImage(back, (width - dw) / 2, (height - dh) / 2, dw, dh);
    } catch {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    return { ok: true, generation, sceneLen: len };
  }
}

// The single i64 result is one frame's immutable boundary metadata:
// status[7:0], scene length[31:8], generation[63:32]. WebAssembly i64
// values surface as signed BigInt, so decode the same bits as unsigned.
export function decodeRenderResult(packed) {
  if (typeof packed !== "bigint") return null;
  const value = BigInt.asUintN(64, packed);
  return {
    status: Number(value & 0xffn),
    sceneLen: Number((value >> 8n) & 0xffffffn),
    generation: Number((value >> 32n) & 0xffffffffn),
  };
}

function color(view, at) {
  const r = view.getUint8(at);
  const g = view.getUint8(at + 1);
  const b = view.getUint8(at + 2);
  const a = view.getUint8(at + 3);
  return `rgba(${r},${g},${b},${a / 255})`;
}

// Paints one text run from the glyph atlas with the reference
// rasterizer's metrics: run size maps to the cell height, the pen
// advances by the manifest advance, the bitmap sits entirely above the
// baseline, and the leftmost column is the highest used row bit. An
// uncovered character throws — nothing substitutes.
export function drawGlyphRun(ctx, glyphs, text, x, y, size, anchorByte) {
  if (text.length === 0 || !(size > 0)) return;
  if (!glyphs) throw new Error("text requires the verified glyph atlas");
  const scale = size / glyphs.cellH;
  const advance = glyphs.advance * scale;
  const chars = [...text];
  const width = chars.length * advance;
  const h = anchorByte & 3;
  const v = (anchorByte >> 2) & 3;
  const left = h === 1 ? x - width / 2 : h === 2 ? x - width : x;
  // v: 0 baseline, 1 middle, 2 top, 3 bottom (descent is zero).
  const top = v === 2 ? y : v === 1 ? y - size / 2 : y - size;
  let pen = left;
  for (const ch of chars) {
    const glyph = glyphs.map.get(ch.codePointAt(0));
    if (!glyph) throw new Error(`no glyph for character ${JSON.stringify(ch)}`);
    for (let row = 0; row < glyph.rows.length; row += 1) {
      for (let col = 0; col < glyphs.cellW; col += 1) {
        if ((glyph.rows[row] >> (glyphs.cellW - 1 - col)) & 1) {
          ctx.fillRect(pen + col * scale, top + row * scale, scale, scale);
        }
      }
    }
    pen += advance;
  }
}

// Fail-closed argument envelope mirroring the reference rasterizer's
// limits (Q8.8 representable range, shared path-vertex buffer). The
// rasterizer range-checks coordinates in device space after applying
// the transform; Canvas2D has no such boundary, so the interpreter
// rejects at the raw-argument level instead — a different (raw, not
// device-space) rule with the same fail-closed outcome for the scenes
// the encoder can produce, which never rely on a transform to bring an
// out-of-range raw coordinate back in range. A violation throws;
// renderPanel converts the throw to PAINT_FAILED and the back buffer
// is discarded, so nothing partial reaches the visible frame.
export const COORD_LIMIT_PX = 32767;
export const MAX_PATH_VERTICES = 512;

function coordGuard(v) {
  if (!Number.isFinite(v) || Math.abs(v) > COORD_LIMIT_PX) {
    throw new Error(`scene coordinate rejected: ${v}`);
  }
  return v;
}

function finiteGuard(v) {
  if (!Number.isFinite(v)) throw new Error(`scene angle rejected: ${v}`);
  return v;
}

// Interprets one encoded scene onto a Canvas2D context; returns the
// number of skipped unknown opcodes. Text paints exclusively from the
// verified glyph atlas — a scene with text and no atlas throws (the
// renderer surfaces it as PAINT_FAILED), never a system-font fallback.
export function interpretScene(view, ctx, glyphs = null) {
  if (view.getUint8(0) !== SCENE_FORMAT_VERSION) return 0;
  let at = 1;
  let unknown = 0;
  ctx.lineJoin = "round";
  while (at + 3 <= view.byteLength) {
    const op = view.getUint8(at);
    const plen = view.getUint16(at + 1, true);
    const p = at + 3;
    if (p + plen > view.byteLength) break;
    const f = (i) => view.getFloat32(p + i * 4, true);
    switch (op) {
      case 0x01:
        ctx.save();
        break;
      case 0x02:
        ctx.restore();
        break;
      case 0x03:
        ctx.translate(coordGuard(f(0)), coordGuard(f(1)));
        break;
      case 0x04:
        ctx.rotate(finiteGuard(f(0)));
        break;
      case 0x10:
        ctx.fillStyle = color(view, p);
        break;
      case 0x11:
        ctx.strokeStyle = color(view, p);
        ctx.lineWidth = coordGuard(view.getFloat32(p + 4, true));
        break;
      case 0x20:
        ctx.beginPath();
        ctx.moveTo(coordGuard(f(0)), coordGuard(f(1)));
        ctx.lineTo(coordGuard(f(2)), coordGuard(f(3)));
        ctx.stroke();
        break;
      case 0x21:
        pointsPath(view, p, plen, 0, ctx);
        ctx.stroke();
        break;
      case 0x22: {
        const mode = view.getUint8(p);
        pointsPath(view, p, plen, 1, ctx);
        ctx.closePath();
        if (mode & 1) ctx.fill();
        if (mode & 2) ctx.stroke();
        break;
      }
      case 0x23: {
        const mode = view.getUint8(p);
        const g = (i) => view.getFloat32(p + 1 + i * 4, true);
        if (mode & 1) ctx.fillRect(coordGuard(g(0)), coordGuard(g(1)), coordGuard(g(2)), coordGuard(g(3)));
        if (mode & 2) ctx.strokeRect(coordGuard(g(0)), coordGuard(g(1)), coordGuard(g(2)), coordGuard(g(3)));
        break;
      }
      case 0x24: {
        const mode = view.getUint8(p);
        const g = (i) => view.getFloat32(p + 1 + i * 4, true);
        ctx.beginPath();
        ctx.arc(coordGuard(g(0)), coordGuard(g(1)), coordGuard(g(2)), 0, Math.PI * 2);
        if (mode & 1) ctx.fill();
        if (mode & 2) ctx.stroke();
        break;
      }
      case 0x25:
        // Scene arc angles match Canvas2D: 0 at +x, positive clockwise
        // in y-down space.
        ctx.beginPath();
        ctx.arc(coordGuard(f(0)), coordGuard(f(1)), coordGuard(f(2)), finiteGuard(f(3)), finiteGuard(f(3) + f(4)), f(4) < 0);
        ctx.stroke();
        break;
      case 0x30: {
        const size = f(0);
        const anchor = view.getUint8(p + 4);
        const x = view.getFloat32(p + 5, true);
        const y = view.getFloat32(p + 9, true);
        const text = new TextDecoder().decode(
          new Uint8Array(view.buffer, view.byteOffset + p + 13, plen - 13),
        );
        drawGlyphRun(ctx, glyphs, text, coordGuard(x), coordGuard(y), coordGuard(size), anchor);
        break;
      }
      case 0x31:
        // Provenance claim for the next text run, consumed by the
        // admission harness; paints nothing. Known vocabulary — see
        // the layer-marker note on the version-skew diagnostic.
        break;
      case 0x40:
        ctx.beginPath();
        ctx.rect(coordGuard(f(0)), coordGuard(f(1)), coordGuard(f(2)), coordGuard(f(3)));
        ctx.clip();
        break;
      case 0x50:
      case 0x51:
        // Layer markers: part of the known vocabulary, painted as
        // no-ops. Z-order and layer structure are enforced by the
        // renderer before commit, and the embedded save/restore
        // envelope carries the state isolation. Counting them as
        // unknown would permanently poison the version-skew diagnostic.
        break;
      default:
        unknown += 1;
        break;
    }
    at = p + plen;
  }
  return unknown;
}

function pointsPath(view, p, plen, headBytes, ctx) {
  ctx.beginPath();
  const n = (plen - headBytes) / 8;
  if (n > MAX_PATH_VERTICES) {
    throw new Error(`scene path rejected: ${n} vertices exceeds ${MAX_PATH_VERTICES}`);
  }
  for (let i = 0; i < n; i++) {
    const x = coordGuard(view.getFloat32(p + headBytes + i * 8, true));
    const y = coordGuard(view.getFloat32(p + headBytes + i * 8 + 4, true));
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
}

// ZYX euler (aerospace) to the body->NED quaternion the state ABI carries.
export function eulerToQuat(rollRad, pitchRad, yawRad) {
  const cr = Math.cos(rollRad / 2);
  const sr = Math.sin(rollRad / 2);
  const cp = Math.cos(pitchRad / 2);
  const sp = Math.sin(pitchRad / 2);
  const cy = Math.cos(yawRad / 2);
  const sy = Math.sin(yawRad / 2);
  return {
    w: cr * cp * cy + sr * sp * sy,
    x: sr * cp * cy - cr * sp * sy,
    y: cr * sp * cy + sr * cp * sy,
    z: cr * cp * sy - sr * sp * cy,
  };
}
