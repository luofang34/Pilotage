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

export const PANEL = { PFD: 0, HSI: 1 };

// Logical drawing space every panel targets (pilotage-instrument-panels
// PANEL_W/PANEL_H); backends scale it to their viewport.
export const LOGICAL_W = 480;
export const LOGICAL_H = 360;

const SCENE_FORMAT_VERSION = 1;
export const STATE_ABI_VERSION = 1;
export const STATE_ABI_SIZE_BY_VERSION = Object.freeze({ 1: 120, 2: 128 });
export const STATE_ABI_SIZE = STATE_ABI_SIZE_BY_VERSION[STATE_ABI_VERSION];
const MAX_WASM_RENDER_STATUS = 8;

// A module missing any required render export is incompatible and must fail
// as an ABI mismatch rather than as a TypeError mid-frame.
const REQUIRED_EXPORTS = [
  "abi_version",
  "init",
  "state_ptr",
  "state_len",
  "scene_ptr",
  "scene_len",
  "render_status",
  "render_generation",
  "set_v_speeds",
  "memory",
];

export async function loadInstruments(wasmUrl, options = {}) {
  const fetchWasm = options.fetch ?? fetch;
  const instantiateStreaming = options.instantiateStreaming ?? WebAssembly.instantiateStreaming;
  const instantiate = options.instantiate ?? WebAssembly.instantiate;
  let exports;
  try {
    const response = await fetchWasm(wasmUrl);
    let instance;
    try {
      ({ instance } = await instantiateStreaming(response, {}));
    } catch {
      // Static servers without the wasm MIME type fall back to ArrayBuffer.
      const bytes = await (await fetchWasm(wasmUrl)).arrayBuffer();
      ({ instance } = await instantiate(bytes, {}));
    }
    exports = instance.exports;
  } catch (error) {
    throw new InstrumentFault(REASON.WASM_LOAD, `instrument wasm load failed: ${error}`);
  }
  for (const name of REQUIRED_EXPORTS) {
    const value = exports[name];
    const validMemory =
      name === "memory" &&
      value !== null &&
      typeof value === "object" &&
      (value.buffer instanceof ArrayBuffer ||
        (typeof SharedArrayBuffer !== "undefined" && value.buffer instanceof SharedArrayBuffer));
    const validFunction = name !== "memory" && typeof value === "function";
    if (!(name in exports) || (!validMemory && !validFunction)) {
      throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument wasm has invalid export ${name}`);
    }
  }
  let abiVersion;
  try {
    abiVersion = exports.abi_version();
  } catch (error) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument ABI query failed: ${error}`);
  }
  if (abiVersion !== STATE_ABI_VERSION) {
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument ABI mismatch: wasm=${abiVersion} js=${STATE_ABI_VERSION}`,
    );
  }
  let initialized;
  try {
    initialized = exports.init();
  } catch (error) {
    throw new InstrumentFault(REASON.INIT_FAILED, `instrument wasm init trapped: ${error}`);
  }
  if (initialized !== 1) {
    throw new InstrumentFault(REASON.INIT_FAILED, `instrument wasm init returned ${initialized}`);
  }
  let stateLen;
  try {
    stateLen = exports.state_len();
  } catch (error) {
    throw new InstrumentFault(REASON.ABI_MISMATCH, `instrument state length query failed: ${error}`);
  }
  if (stateLen !== STATE_ABI_SIZE) {
    throw new InstrumentFault(
      REASON.ABI_MISMATCH,
      `instrument state size mismatch: wasm=${stateLen} js=${STATE_ABI_SIZE}`,
    );
  }
  return new InstrumentModule(exports, options);
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
  // options.createCanvas injects the back-buffer factory (tests pass a
  // recording double; the browser default is a plain offscreen element).
  constructor(exports, { createCanvas } = {}) {
    this.exports = exports;
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

  setVSpeeds(vs0, vs, vfe, vno, vne) {
    try {
      this.exports.set_v_speeds(vs0, vs, vfe, vno, vne);
      return { ok: true };
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
  }

  // state: see writeState field handling below; absent groups may be
  // omitted entirely.
  writeState(state) {
    try {
      const ptr = this.exports.state_ptr();
      const len = this.exports.state_len();
      if (len !== STATE_ABI_SIZE) return { ok: false, reason: REASON.ABI_MISMATCH };
      const view = new DataView(this.exports.memory.buffer, ptr, len);
      const f = (off, v) => view.setFloat32(off, v ?? NaN, true);
      const b = (off, v) => view.setUint8(off, v);

      view.setUint32(0, STATE_ABI_VERSION, true);
      const att = state.attitude;
      f(4, att?.quat?.w ?? 1);
      f(8, att?.quat?.x ?? 0);
      f(12, att?.quat?.y ?? 0);
      f(16, att?.quat?.z ?? 0);
      f(20, att?.rates?.[0] ?? 0);
      f(24, att?.rates?.[1] ?? 0);
      f(28, att?.rates?.[2] ?? 0);
      const kin = state.kinematics;
      f(32, kin?.posNed?.[0] ?? 0);
      f(36, kin?.posNed?.[1] ?? 0);
      f(40, kin?.posNed?.[2] ?? 0);
      f(44, kin?.velNed?.[0] ?? 0);
      f(48, kin?.velNed?.[1] ?? 0);
      f(52, kin?.velNed?.[2] ?? 0);
      f(56, state.air?.iasMps ?? NaN);
      f(60, state.air?.baroHpa ?? NaN);
      f(64, att ? att.ageMs : NaN);
      f(68, kin ? kin.ageMs : NaN);
      f(72, state.air ? state.air.ageMs : NaN);
      f(76, state.nav ? state.nav.ageMs : NaN);
      f(80, state.wind ? state.wind.ageMs : NaN);
      b(84, state.quality ?? 0);
      const valid = state.valid ?? {};
      b(
        85,
        (valid.attitude ?? true ? 1 : 0) |
          (valid.rates ?? true ? 2 : 0) |
          (valid.position ?? true ? 4 : 0) |
          (valid.velocity ?? true ? 8 : 0),
      );
      b(86, state.nav?.source ?? 0);
      b(87, state.nav?.fromto ?? 0);
      f(88, state.nav?.courseRad ?? 0);
      f(92, state.nav?.cdiDots ?? 0);
      f(96, state.nav?.vdevDots ?? NaN);
      f(100, state.nav?.distNm ?? NaN);
      f(104, state.selections?.headingBugRad ?? 0);
      f(108, state.selections?.altitudeSelM ?? NaN);
      f(112, state.wind?.fromRad ?? 0);
      f(116, state.wind?.speedMps ?? 0);
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
    if (!Number.isFinite(width) || width <= 0 || !Number.isFinite(height) || height <= 0) {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    let status;
    try {
      status = this.exports.render_status(panel);
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (!Number.isInteger(status) || status < 0 || status > MAX_WASM_RENDER_STATUS) {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (status !== 0) return { ok: false, reason: status };
    let view;
    let generation;
    try {
      const len = this.exports.scene_len();
      const ptr = this.exports.scene_ptr();
      view = new DataView(this.exports.memory.buffer, ptr, len);
      generation = this.exports.render_generation(panel);
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (!Number.isInteger(generation)) return { ok: false, reason: REASON.RENDER_TRAP };
    let structurallyValid;
    try {
      structurallyValid = validateSceneStructure(view);
    } catch {
      return { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (!structurallyValid) return { ok: false, reason: REASON.SCENE_FRAMING };
    let back;
    try {
      back = this.back.get(panel);
      if (!back) {
        back = this.createCanvas(LOGICAL_W, LOGICAL_H);
        this.back.set(panel, back);
      }
      // Re-assigning width resets the back context wholesale (state
      // stack, transform, clip), so an unbalanced save/clip in one frame
      // cannot leak into the next.
      back.width = LOGICAL_W;
      back.height = LOGICAL_H;
      const bctx = back.getContext("2d");
      bctx.fillStyle = "#000";
      bctx.fillRect(0, 0, LOGICAL_W, LOGICAL_H);
      this.unknownOpcodes += interpretScene(view, bctx);
    } catch {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    try {
      const scale = Math.min(width / LOGICAL_W, height / LOGICAL_H);
      const dw = LOGICAL_W * scale;
      const dh = LOGICAL_H * scale;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, width, height);
      ctx.drawImage(back, (width - dw) / 2, (height - dh) / 2, dw, dh);
    } catch {
      return { ok: false, reason: REASON.PAINT_FAILED };
    }
    return { ok: true, generation: generation >>> 0 };
  }
}

function color(view, at) {
  const r = view.getUint8(at);
  const g = view.getUint8(at + 1);
  const b = view.getUint8(at + 2);
  const a = view.getUint8(at + 3);
  return `rgba(${r},${g},${b},${a / 255})`;
}

const H_ALIGN = ["left", "center", "right"];
const V_ALIGN = ["alphabetic", "middle", "top", "bottom"];

// Interprets one encoded scene onto a Canvas2D context; returns the
// number of skipped unknown opcodes.
export function interpretScene(view, ctx) {
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
        ctx.translate(f(0), f(1));
        break;
      case 0x04:
        ctx.rotate(f(0));
        break;
      case 0x10:
        ctx.fillStyle = color(view, p);
        break;
      case 0x11:
        ctx.strokeStyle = color(view, p);
        ctx.lineWidth = view.getFloat32(p + 4, true);
        break;
      case 0x20:
        ctx.beginPath();
        ctx.moveTo(f(0), f(1));
        ctx.lineTo(f(2), f(3));
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
        if (mode & 1) ctx.fillRect(g(0), g(1), g(2), g(3));
        if (mode & 2) ctx.strokeRect(g(0), g(1), g(2), g(3));
        break;
      }
      case 0x24: {
        const mode = view.getUint8(p);
        const g = (i) => view.getFloat32(p + 1 + i * 4, true);
        ctx.beginPath();
        ctx.arc(g(0), g(1), g(2), 0, Math.PI * 2);
        if (mode & 1) ctx.fill();
        if (mode & 2) ctx.stroke();
        break;
      }
      case 0x25:
        // Scene arc angles match Canvas2D: 0 at +x, positive clockwise
        // in y-down space.
        ctx.beginPath();
        ctx.arc(f(0), f(1), f(2), f(3), f(3) + f(4), f(4) < 0);
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
        ctx.font = `bold ${size}px system-ui, sans-serif`;
        ctx.textAlign = H_ALIGN[anchor & 3] ?? "left";
        ctx.textBaseline = V_ALIGN[(anchor >> 2) & 3] ?? "alphabetic";
        ctx.fillText(text, x, y);
        break;
      }
      case 0x40:
        ctx.beginPath();
        ctx.rect(f(0), f(1), f(2), f(3));
        ctx.clip();
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
  for (let i = 0; i < n; i++) {
    const x = view.getFloat32(p + headBytes + i * 8, true);
    const y = view.getFloat32(p + headBytes + i * 8 + 4, true);
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
