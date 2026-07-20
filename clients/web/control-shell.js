// The thin JS half of the control vertical slice. It owns ONLY Gamepad/keyboard
// sampling and the wasm plumbing: it copies a raw device sample into the
// control-runtime's input buffer, calls evaluate() once, and reads a semantic
// plan back out. All mapping, response curves, the gimbal quasimode, masking,
// edge detection, lease planning, and runtime state live in the Rust/WASM
// runtime (pilotage-control-web) behind that one call — there is no mapping
// table, controller index, deadzone, or expo here.

// Input buffer layout (must match clients/web-control/src/wasm.rs):
//   axes f32[8] | button values f32[24] | pressed-bitset u32.
const MAX_AXES = 8;
const MAX_BUTTONS = 24;
const IN_AXES = 0;
const IN_VALUES = IN_AXES + MAX_AXES * 4;
const IN_PRESSED = IN_VALUES + MAX_BUTTONS * 4;
// Output buffer layout: flags u32 | motion f32[4] | gimbal f32[2].
const OUT_FLAGS = 0;
const OUT_MOTION = 4;
const OUT_GIMBAL = OUT_MOTION + 4 * 4;

const FLAG_MOTION = 1;
const FLAG_GIMBAL = 1 << 1;
const FLAG_RECENTER = 1 << 2;
const FLAG_ARM = 1 << 3;
const FLAG_DISARM = 1 << 4;
const LEASE_SHIFT = 8; // bits 8..9: 0 none, 1 request, 2 release.

const MODE_IDS = { "quad-pilot": 0, "quad-cruise": 1, fpv: 2, rover: 3 };

// Keyboard is a first-class raw source, not a second mapping: each key sets a
// Standard-Gamepad axis/button position, and the SAME wasm runtime maps it.
const KEY_AXES = [
  { key: "KeyS", axis: 1, value: 1 },
  { key: "KeyW", axis: 1, value: -1 },
  { key: "KeyD", axis: 0, value: 1 },
  { key: "KeyA", axis: 0, value: -1 },
  { key: "ArrowDown", axis: 3, value: 1 },
  { key: "ArrowUp", axis: 3, value: -1 },
  { key: "ArrowRight", axis: 2, value: 1 },
  { key: "ArrowLeft", axis: 2, value: -1 },
];
const KEY_BUTTONS = [
  { key: "Enter", button: 9 }, // arm
  { key: "Backspace", button: 8 }, // disarm
];

/** Loads the control-runtime wasm and bootstraps it through the normal
 *  activation path: compile the built-in default profile bytes, then activate.
 *  There is no privileged default entry point. */
export async function loadControlShell(wasmUrl, options = {}) {
  const bindings = await (options.loadBindings ?? (() => import("./control-runtime.js")))();
  const init = options.initializeBindings ?? bindings.default;
  const wasm = await init({ module_or_path: wasmUrl });
  if (!(wasm?.memory?.buffer instanceof ArrayBuffer)) {
    throw new Error("control binding has invalid memory");
  }
  const control = new bindings.WebControl();
  const revision = control.activate(bindings.default_profile());
  if (revision === 0) {
    throw new Error("the built-in default profile failed to compile");
  }
  return new ControlShell(wasm, control);
}

/** A live control runtime plus its wasm memory, driving one tick at a time. */
export class ControlShell {
  #wasm;
  #control;

  constructor(wasm, control) {
    this.#wasm = wasm;
    this.#control = control;
  }

  /** The session activation revision (advances on each profile install). */
  activationRevision() {
    return this.#control.activation_revision();
  }

  /** Compiles and activates candidate profile bytes through the same seam the
   *  bootstrap uses. Returns the new revision, or 0 if the candidate was
   *  rejected (the currently active profile stays active). */
  activate(candidateBytes) {
    return this.#control.activate(candidateBytes);
  }

  /** Evaluates one tick from a connected gamepad. */
  tickFromPad(pad, session) {
    const axisCount = this.#writePad(pad);
    return this.#evaluate(axisCount, MAX_BUTTONS, session);
  }

  /** Evaluates one tick from the keyboard fallback (no gamepad present). */
  tickFromKeys(keySet, session) {
    const buttonCount = this.#writeKeys(keySet);
    return this.#evaluate(4, buttonCount, session);
  }

  #inputViews() {
    const buffer = this.#wasm.memory.buffer;
    const base = this.#control.input_ptr();
    return {
      axes: new Float32Array(buffer, base + IN_AXES, MAX_AXES),
      values: new Float32Array(buffer, base + IN_VALUES, MAX_BUTTONS),
      pressed: new Uint32Array(buffer, base + IN_PRESSED, 1),
    };
  }

  #writePad(pad) {
    const view = this.#inputViews();
    view.axes.fill(0);
    view.values.fill(0);
    let bits = 0;
    const axisCount = Math.min(pad.axes.length, MAX_AXES);
    for (let i = 0; i < axisCount; i += 1) view.axes[i] = pad.axes[i];
    const buttons = pad.buttons ?? [];
    for (let i = 0; i < Math.min(buttons.length, MAX_BUTTONS); i += 1) {
      view.values[i] = buttons[i]?.value ?? 0;
      if (buttons[i]?.pressed) bits |= 1 << i;
    }
    view.pressed[0] = bits >>> 0;
    return axisCount;
  }

  #writeKeys(keySet) {
    const view = this.#inputViews();
    view.axes.fill(0);
    view.values.fill(0);
    let bits = 0;
    for (const { key, axis, value } of KEY_AXES) {
      if (keySet.has(key)) view.axes[axis] = value;
    }
    for (const { key, button } of KEY_BUTTONS) {
      if (keySet.has(key)) {
        view.values[button] = 1;
        bits |= 1 << button;
      }
    }
    view.pressed[0] = bits >>> 0;
    return 10;
  }

  #evaluate(axisCount, buttonCount, session) {
    const mode = MODE_IDS[session.mode] ?? 0;
    const flags =
      (session.connected ? 1 : 0) |
      (session.leaseGranted ? 1 << 1 : 0) |
      (session.leaseDenied ? 1 << 2 : 0);
    const result = this.#control.evaluate(axisCount, buttonCount, mode, session.nowMs, flags);
    return this.#readPlan(result);
  }

  #readPlan(flags) {
    const buffer = this.#wasm.memory.buffer;
    const base = this.#control.output_ptr();
    const motionView = new Float32Array(buffer, base + OUT_MOTION, 4);
    const gimbalView = new Float32Array(buffer, base + OUT_GIMBAL, 2);
    const motion =
      flags & FLAG_MOTION
        ? { roll: motionView[0], pitch: motionView[1], throttle: motionView[2], yaw: motionView[3] }
        : null;
    const gimbal =
      flags & FLAG_GIMBAL
        ? { pitch: gimbalView[0], yaw: gimbalView[1], recenter: (flags & FLAG_RECENTER) !== 0 }
        : null;
    const leaseCode = (flags >>> LEASE_SHIFT) & 0b11;
    return {
      motion,
      gimbal,
      arm: (flags & FLAG_ARM) !== 0,
      disarm: (flags & FLAG_DISARM) !== 0,
      lease: leaseCode === 1 ? "request" : leaseCode === 2 ? "release" : null,
    };
  }
}
