// Structural guard: no first-party browser module may hold input-mapping
// logic. Device mapping (keyboard AND gamepad), controller identity tables,
// response curves (deadzone/expo), and the gimbal state machine live in the
// Rust/WASM runtime behind one evaluate() call, sourced from device-profile
// DATA — if any table reappears in shell JavaScript, future profile
// restoration would again mean an architectural rewrite instead of a source
// change. This scans EVERY module in clients/web (not only main.js), so a
// mapping table cannot dodge the guard by moving files.

import { readFileSync, readdirSync } from "node:fs";

// Generated wasm bindings are build artifacts, not first-party shell code.
const GENERATED = new Set(["control-runtime.js", "instrument-runtime.js"]);

const dir = new URL("./", import.meta.url);
const modules = readdirSync(dir)
  .filter((name) => name.endsWith(".js") && !GENERATED.has(name))
  .sort();

let failures = 0;
function forbid(name, source, label, pattern) {
  const match = source.match(pattern);
  if (match) {
    failures += 1;
    console.error(`FAIL - ${name} must not contain ${label} (found ${JSON.stringify(match[0])})`);
  }
}

// Patterns any first-party module is banned from holding.
const BANNED = [
  ["a deadzone", /\bdeadzone\b/i],
  ["an expo curve", /\bexpo\b/],
  ["a key mapping table", /KEY_AXES|KEY_BUTTONS|DRIVE_KEYS/],
  ["a key-to-axis binding literal", /key:\s*"[^"]+",\s*(axis|button):/],
  ["a flight-scheme mapping table", /FLIGHT_SCHEMES/],
  ["a stick shaper", /stickShaper/],
  ["a controller-profile mapping table", /CONTROLLER_PROFILES|forwardAxis|turnAxis/],
  ["gamepad identity parsing", /parseGamepadId|vendorId\s*=/],
  [
    "a retired gimbal mapping function",
    /gimbal(AxesFromGamepad|MaskedView|ModifierHeld|FramePlan|ResetEdge|LeasePlan|WheelRates)/,
  ],
  ["an import from the retired gimbal-input module", /gimbal-input/],
];

for (const name of modules) {
  const source = readFileSync(new URL(`./${name}`, dir), "utf8");
  for (const [label, pattern] of BANNED) {
    forbid(name, source, label, pattern);
  }
}
console.log(`scanned ${modules.length} modules for mapping logic`);

// And the shell must still drive the runtime through the one seam.
const main = readFileSync(new URL("./main.js", dir), "utf8");
function require(label, pattern) {
  if (pattern.test(main)) {
    console.log(`ok   - main.js ${label}`);
  } else {
    failures += 1;
    console.error(`FAIL - main.js must ${label}`);
  }
}
require("imports the control shell", /from "\.\/control-shell\.js"/);
require("evaluates one tick through the runtime", /tickFromPad|tickFromKeys/);
require("resolves pad identity through the runtime selector", /selectDevice/);
require("forwards key transitions to the runtime", /keyEvent/);
// A same-session resume must hand the runtime a FRESH session generation
// before its loop restarts: the generation prime is what re-seeds the
// discrete edge baselines and re-enters neutral-activation recovery.
require(
  "advances the control generation before a resumed loop starts",
  /function completePendingResume[\s\S]*?controlGeneration \+ 1[\s\S]*?startControlLoop/,
);

if (failures > 0) {
  console.error(`${failures} structural violation(s)`);
  process.exit(1);
}
console.log("browser control structural contract passed");
