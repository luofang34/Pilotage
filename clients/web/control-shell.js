// The thin JS half of the control vertical slice. It owns ONLY Gamepad/keyboard
// sampling and the wasm plumbing: it copies a raw device sample into the
// control-runtime's input buffer, calls evaluate() once, and reads a semantic
// plan back out. All device mapping, response curves, the gimbal quasimode,
// masking, edge detection, lease planning, and runtime state live in the
// Rust/WASM runtime (pilotage-control-web) behind that one call — this file
// holds no mapping table, controller index, or response-curve constant.

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
const FLAG_CAPTURE = 1 << 5;
const LEASE_SHIFT = 8; // bits 8..9: gimbal lease 0 none, 1 request, 2 release.
const MOTION_LEASE_SHIFT = 10; // bits 10..11: motion lease, same encoding.

const MODE_IDS = { "quad-pilot": 0, "quad-cruise": 1, fpv: 2, rover: 3 };

// Raw-sample source selector for evaluate(): pad reads the input buffer
// through the selected device profile; keys synthesize from the held-key
// state the runtime tracks. There is NO key or axis table here — keyboard
// and gamepad mappings are device-profile DATA inside the runtime, and the
// shell forwards Gamepad.id strings and key events verbatim.
const SOURCE_PAD = 0;
const SOURCE_KEYS = 1;

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

  // Cached typed-array views over wasm memory, rebuilt only when the backing
  // buffer changes (a wasm memory grow detaches old views); a steady tick
  // reuses them and allocates nothing.
  #views;
  #viewBuffer;

  /** The session activation revision (advances on each profile install). */
  activationRevision() {
    return this.#control.activation_revision();
  }

  /** The active profile's identity string. */
  profileId() {
    return this.#control.profile_id();
  }

  /** The active profile DOCUMENT revision (ADR-0007/0009) — the value carried
   *  on control frames as profile_revision, distinct from the activation
   *  epoch. */
  profileRevision() {
    return this.#control.profile_revision();
  }

  /** The active profile's content digest as a lowercase hex string. */
  profileDigest() {
    return Array.from(this.#control.profile_digest(), (byte) =>
      byte.toString(16).padStart(2, "0"),
    ).join("");
  }

  /** Compiles and activates candidate profile bytes through the same seam the
   *  bootstrap uses. Returns the new revision, or 0 if the candidate was
   *  rejected (the currently active profile stays active). */
  activate(candidateBytes) {
    return this.#control.activate(candidateBytes);
  }

  /** Resolves a Gamepad.id through the runtime's shared device selector.
   *  Returns "exact", "fallback", or null when refused (an ambiguous
   *  registry fails closed: that pad's ticks drive nothing). */
  selectDevice(gamepadId) {
    const outcome = this.#control.select_device(gamepadId ?? "");
    return outcome === 1 ? "exact" : outcome === 2 ? "fallback" : null;
  }

  /** The selected pad profile's human-readable label (empty when refused). */
  deviceLabel() {
    return this.#control.device_label();
  }

  /** Forwards one canonical key transition (letters lower-cased) to the
   *  runtime's held-key state. */
  keyEvent(key, pressed) {
    this.#control.key_event(key, pressed);
  }

  /** Whether the keyboard profile binds this canonical key — the shell's
   *  capture filter, answered from profile data. */
  boundKey(key) {
    return this.#control.key_is_bound(key);
  }

  /** Drops every held key (window blur or session teardown). */
  clearKeys() {
    this.#control.clear_keys();
  }

  /** Evaluates one tick from a connected gamepad. */
  tickFromPad(pad, session) {
    const axisCount = this.#writePad(pad);
    return this.#evaluate(axisCount, MAX_BUTTONS, session, SOURCE_PAD);
  }

  /** Evaluates one tick from the keyboard (no gamepad present); the sample
   *  synthesizes inside the runtime from its held-key state. */
  tickFromKeys(session) {
    return this.#evaluate(0, 0, session, SOURCE_KEYS);
  }

  #memoryViews() {
    const buffer = this.#wasm.memory.buffer;
    if (this.#viewBuffer === buffer && this.#views) return this.#views;
    const inBase = this.#control.input_ptr();
    const outBase = this.#control.output_ptr();
    this.#views = {
      axes: new Float32Array(buffer, inBase + IN_AXES, MAX_AXES),
      values: new Float32Array(buffer, inBase + IN_VALUES, MAX_BUTTONS),
      pressed: new Uint32Array(buffer, inBase + IN_PRESSED, 1),
      motion: new Float32Array(buffer, outBase + OUT_MOTION, 4),
      gimbal: new Float32Array(buffer, outBase + OUT_GIMBAL, 2),
    };
    this.#viewBuffer = buffer;
    return this.#views;
  }

  #writePad(pad) {
    const view = this.#memoryViews();
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

  #evaluate(axisCount, buttonCount, session, source) {
    const mode = MODE_IDS[session.mode] ?? 0;
    const flags =
      (session.connected ? 1 : 0) |
      (session.leaseGranted ? 1 << 1 : 0) |
      (session.leaseDenied ? 1 << 2 : 0) |
      (session.motionGranted ? 1 << 3 : 0) |
      (session.motionDenied ? 1 << 4 : 0) |
      (session.motionRecovered ? 1 << 5 : 0);
    const result = this.#control.evaluate(
      axisCount,
      buttonCount,
      mode,
      session.nowMs,
      flags,
      session.generation >>> 0,
      source,
    );
    return this.#readPlan(result);
  }

  #readPlan(flags) {
    const view = this.#memoryViews();
    const motion =
      flags & FLAG_MOTION
        ? { roll: view.motion[0], pitch: view.motion[1], throttle: view.motion[2], yaw: view.motion[3] }
        : null;
    const gimbal =
      flags & FLAG_GIMBAL
        ? { pitch: view.gimbal[0], yaw: view.gimbal[1], recenter: (flags & FLAG_RECENTER) !== 0 }
        : null;
    const leaseCode = (flags >>> LEASE_SHIFT) & 0b11;
    const motionLeaseCode = (flags >>> MOTION_LEASE_SHIFT) & 0b11;
    const leaseName = (code) => (code === 1 ? "request" : code === 2 ? "release" : null);
    return {
      motion,
      gimbal,
      arm: (flags & FLAG_ARM) !== 0,
      disarm: (flags & FLAG_DISARM) !== 0,
      captureActive: (flags & FLAG_CAPTURE) !== 0,
      lease: leaseName(leaseCode),
      motionLease: leaseName(motionLeaseCode),
    };
  }
}
