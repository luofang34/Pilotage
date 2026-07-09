// Browser backend for the instrument runtime (ADR-0017).
//
// Loads the pilotage-instruments-web WASM module (built by
// scripts/build-web-instruments.sh), writes packed aircraft state into its
// linear memory (mirroring pilotage-instrument-state/src/abi.rs exactly),
// and interprets the returned scene-command bytes onto a Canvas2D.
//
// The interpreter is one backend of the versioned scene IR; unknown
// opcodes are skipped and counted, never fatal (ADR-0017).

export const PANEL = { PFD: 0, HSI: 1 };

const SCENE_FORMAT_VERSION = 1;
const STATE_ABI_VERSION = 1;

export async function loadInstruments(wasmUrl) {
  const response = await fetch(wasmUrl);
  let instance;
  try {
    ({ instance } = await WebAssembly.instantiateStreaming(response, {}));
  } catch {
    // Static servers without the wasm MIME type fall back to ArrayBuffer.
    const bytes = await (await fetch(wasmUrl)).arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(bytes, {}));
  }
  const exports = instance.exports;
  if (exports.abi_version() !== STATE_ABI_VERSION) {
    throw new Error(`instrument ABI mismatch: wasm=${exports.abi_version()} js=${STATE_ABI_VERSION}`);
  }
  if (exports.init() !== 1) {
    throw new Error("instrument wasm init failed");
  }
  return new InstrumentModule(exports);
}

export class InstrumentModule {
  constructor(exports) {
    this.exports = exports;
    this.unknownOpcodes = 0;
  }

  setVSpeeds(vs0, vs, vfe, vno, vne) {
    this.exports.set_v_speeds(vs0, vs, vfe, vno, vne);
  }

  // state: see writeState field handling below; absent groups may be
  // omitted entirely.
  writeState(state) {
    const ptr = this.exports.state_ptr();
    const len = this.exports.state_len();
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
  }

  // Renders `panel` and paints it onto ctx2d, scaling the 480x360
  // logical space to fit (letterboxed).
  renderTo(ctx, panel, width, height) {
    const len = this.exports.render(panel);
    if (len === 0) return false;
    const bytes = new DataView(this.exports.memory.buffer, this.exports.scene_ptr(), len);
    const scale = Math.min(width / 480, height / 360);
    ctx.save();
    ctx.setTransform(scale, 0, 0, scale, (width - 480 * scale) / 2, (height - 360 * scale) / 2);
    this.unknownOpcodes += interpretScene(bytes, ctx);
    ctx.restore();
    return true;
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
