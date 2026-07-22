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

const SIZE_CAPS = {
  "action-tracker.js": 90,
  "authority-stream.js": 17,
  "bootstrap.js": 88,
  "calibration.js": 285,
  "connect-authority.js": 63,
  "control-edges.js": 31,
  "control-gate.js": 39,
  "control-shell.js": 341,
  "datagram-control.js": 80,
  "instrument-health.js": 395,
  "instruments.js": 761,
  "layout.js": 55,
  "lease-executor.js": 14,
  "lease-release.js": 67,
  "main.js": 2290,
  "reconnect.js": 150,
  "resume-control.js": 59,
  "session-discovery.js": 68,
  "snapshot-association.js": 266,
  "telemetry-display.js": 75,
  "telemetry-ingress.js": 689,
  "transport-session.js": 120,
  "turn-derivation.js": 101,
  "typed-command.js": 131,
  "uni-stream-accept.js": 78,
  "uni-stream.js": 58,
  "video-diagnostics.js": 29,
  "video-h264.js": 226,
  "video-identity.js": 291,
  "video-routing.js": 37,
  "wire-bounds.js": 120,
  "wire.js": 1143,
};

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
  ["a hardcoded arm-control naming", /Options\/Enter|Create\/Backspace/],
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
  const lines = source.match(/\n/g)?.length ?? 0;
  if (SIZE_CAPS[name] === undefined || lines > SIZE_CAPS[name]) {
    failures += 1;
    console.error(`FAIL - ${name} has ${lines} lines (cap ${SIZE_CAPS[name] ?? "missing"})`);
  }
}
console.log(`scanned ${modules.length} modules for mapping logic`);

const leaseRequestOwners = modules.filter(
  (name) =>
    name !== "wire.js" &&
    readFileSync(new URL(`./${name}`, dir), "utf8").includes("encodeLeaseRequestEnvelope"),
);
if (leaseRequestOwners.join(",") !== "lease-executor.js") {
  failures += 1;
  console.error(
    `FAIL - lease requests must have one encoder owner (got ${leaseRequestOwners.join(",")})`,
  );
}

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
// Every datagram run re-seeds discrete edges and derives recovery from the
// runtime-owned authority table before its loop starts.
require(
  "begins a control run through the runtime",
  /function startControlLoop[\s\S]*?beginControlRun\(\)/,
);
// Operator-facing arm/disarm names come from the runtime (profile data),
// so a rebound control or a different device renames its own hint.
require("derives the arm/disarm hint from the runtime", /armHint\(\)/);
// While a live session has no control run, input still evaluates through
// the runtime so every press answers loudly instead of vanishing.
require("watches suspended input through the runtime", /startSuspendedPressWatch\(/);

if (failures > 0) {
  console.error(`${failures} structural violation(s)`);
  process.exit(1);
}
console.log("browser control structural contract passed");
