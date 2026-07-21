// The browser half of the typed-control E2E chain (CTRL-01): physical input
// (keyboard AND gamepad) through the REAL compiled wasm runtime, scaled onto
// an advertised velocity envelope, wire-encoded — asserting byte-identically
// against the shared fixture whose bytes the Rust session-engine E2E
// (crates/pilotage-session/tests/typed_wire_e2e.rs) decodes and drives to
// the adapter boundary. One fixture, both ends: the chain cannot drift.
//
// Build the wasm first: scripts/build-web-instruments.sh

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";
import {
  buildGimbalRateIntent,
  buildVelocityIntent,
  intentCapabilityFor,
} from "./typed-command.js";
import { CONTROL_ACTION, MODE_TARGET, encodeControlFrameEnvelope } from "./wire.js";

const fixture = JSON.parse(
  readFileSync(new URL("../web-control/typed-frame-fixture.json", import.meta.url), "utf8"),
);
const wasmBytes = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));
const shell = await loadControlShell(wasmBytes);

let failures = 0;
function check(name, ok, got) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}${got === undefined ? "" : ` (got ${JSON.stringify(got)})`}`);
  }
}

const session = {
  generation: 1,
  mode: "quad-pilot",
  connected: true,
  leaseGranted: true,
  leaseDenied: false,
  motionGranted: true,
  motionDenied: false,
  motionRecovered: true,
  nowMs: 100_000,
};
const capability = fixture.capability;

// Keyboard chain: W (climb) through the real wasm to the typed envelope.
shell.keyEvent("w", true);
const keyPlan = shell.tickFromKeys(session);
const keyVelocity = buildVelocityIntent(keyPlan.motion, "quad-pilot", capability);
shell.clearKeys();
check("keyboard W commands full climb in the advertised envelope", keyVelocity.vz === -1.5, keyVelocity);

const bytes = encodeControlFrameEnvelope({
  sessionId: 7,
  vehicleId: 1n,
  scope: "vehicle.motion",
  generation: 4n,
  sequence: 42,
  sampledAtNanos: 123_456_789n,
  profileRevision: shell.profileRevision(),
  activationRevision: shell.activationRevision(),
  velocity: keyVelocity,
  actions: [
    { action: CONTROL_ACTION.arm },
    { action: CONTROL_ACTION.modeRequest, modeTarget: MODE_TARGET.fpvDirect },
  ],
});
const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
check("the typed envelope matches the shared fixture byte for byte", hex === fixture.envelopeHex, hex);

// Gamepad chain: left stick hard right = full yaw rate in the pilot scheme.
const pad = {
  axes: [1, 0, 0, 0],
  buttons: Array.from({ length: 16 }, () => ({ pressed: false, value: 0 })),
};
const padPlan = shell.tickFromPad(pad, session);
const padVelocity = buildVelocityIntent(padPlan.motion, "quad-pilot", capability);
check(
  "gamepad LX commands full yaw rate in the advertised envelope",
  padVelocity.yawRate === fixture.gamepadVelocity.yawRate && padVelocity.yawRate === 0.9,
  padVelocity,
);

// Fail closed: no advertisement, no intent (the caller must not send).
check(
  "no velocity advertisement yields no intent",
  buildVelocityIntent(keyPlan.motion, "quad-pilot", null) === null,
);
check(
  "capability lookup misses an unadvertised scope",
  intentCapabilityFor([{ scope: "vehicle.motion", intents: [] }], "vehicle.motion", 1) === null,
);

// Gimbal rates scale by the advertised angular envelope.
const gimbalRate = buildGimbalRateIntent({ pitch: -1, yaw: 0.5 }, { maxAngular: 0.8 });
check(
  "gimbal rates scale by the advertised envelope",
  gimbalRate.pitchRate === -0.8 && gimbalRate.yawRate === 0.4,
  gimbalRate,
);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("typed-command chain passed (browser half)");
