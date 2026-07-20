// Structural guard: main.js must stay a thin shell. It may sample the Gamepad
// API, touch the DOM and WebTransport, and execute the runtime's plan — but it
// must hold NO device mapping table, controller-index mapping, response curve
// (deadzone/expo), or gimbal state machine. Those live in the Rust/WASM
// runtime behind one evaluate() call. If any reappears here, future profile
// restoration would again mean an architectural rewrite instead of a source
// change; this test fails closed so that regression is loud.

import { readFileSync } from "node:fs";

const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");

let failures = 0;
function forbid(label, pattern) {
  const match = main.match(pattern);
  if (match) {
    failures += 1;
    console.error(`FAIL - main.js must not contain ${label} (found ${JSON.stringify(match[0])})`);
  } else {
    console.log(`ok   - main.js holds no ${label}`);
  }
}
function require(label, pattern) {
  if (pattern.test(main)) {
    console.log(`ok   - main.js ${label}`);
  } else {
    failures += 1;
    console.error(`FAIL - main.js must ${label}`);
  }
}

// Response curves belong to the runtime, never the shell.
forbid("a deadzone", /\bdeadzone\b/);
forbid("an expo curve", /\bexpo\b/);
// Per-mode / per-device mapping tables belong to the runtime.
forbid("a flight-scheme mapping table", /FLIGHT_SCHEMES/);
forbid("a stick shaper", /stickShaper/);
forbid("a controller-profile mapping table", /CONTROLLER_PROFILES|forwardAxis|turnAxis/);
// The gimbal quasimode / mapping is the runtime's, not the shell's. (Network
// lease STATE like gimbalLeaseGranted is fine — the runtime plans, the shell
// only tracks the grant and executes; the retired mapping FUNCTIONS are not.)
forbid(
  "a retired gimbal mapping function",
  /gimbal(AxesFromGamepad|MaskedView|ModifierHeld|FramePlan|ResetEdge|LeasePlan|WheelRates)/,
);
forbid("an import from the retired gimbal-input module", /gimbal-input/);

// And it must drive the runtime through the one seam.
require("imports the control shell", /from "\.\/control-shell\.js"/);
require("evaluates one tick through the runtime", /tickFromPad|tickFromKeys/);

if (failures > 0) {
  console.error(`${failures} structural violation(s)`);
  process.exit(1);
}
console.log("main.js structural contract passed");
