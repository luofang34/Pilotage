// Environment-neutral singleton bootstrap for the instrument wasm
// bindings the feeder wrappers delegate to (#252). The generated init
// guards against double-instantiation, so this composes with
// instruments.js loading the same module for rendering: whichever
// importer runs first instantiates, and both see one linear memory.
//
// A failed load leaves `bindings` null instead of failing module
// evaluation: the boot contract keeps video usable when the instrument
// wasm is unavailable, the panels report unavailable through the
// instruments module's own fail-visible path, and the feeder wrappers
// degrade to their fail-closed nothing-admitted behavior.

import initModule, * as generated from "./instrument-runtime.js";

let loaded = null;
let loadError = null;
// Boot fault injection: lets the degraded-path suite pin the
// wasm-absent contract in a plain node run instead of a browser rig.
const disabled =
  typeof process !== "undefined" && process.env?.PILOTAGE_FEEDER_WASM_DISABLE === "1";
try {
  if (disabled) throw new Error("feeder wasm disabled by PILOTAGE_FEEDER_WASM_DISABLE");
  const wasmUrl = new URL("./instrument-runtime_bg.wasm", import.meta.url);
  if (typeof process !== "undefined" && process.versions?.node) {
    const { readFileSync } = await import("node:fs");
    generated.initSync({ module: readFileSync(wasmUrl) });
  } else {
    await initModule({ module_or_path: wasmUrl });
  }
  loaded = generated;
} catch (error) {
  loadError = error;
}

export const bindings = loaded;
export const feederLoadError = loadError;

// Freezes a marshalled snapshot in place, matching the immutability the
// pre-wasm feeder promised its consumers.
export function deepFreeze(value) {
  if (value === null || typeof value !== "object") return value;
  for (const key of Object.keys(value)) deepFreeze(value[key]);
  return Object.freeze(value);
}
