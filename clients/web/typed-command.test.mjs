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
  buildAttitudeThrustIntent,
  buildGimbalRateIntent,
  buildVelocityIntent,
  integrateHeading,
  intentCapabilityFor,
  wrapHeading,
} from "./typed-command.js";
import {
  CONTROL_ACTION,
  MODE_TARGET,
  encodeControlActionCommandEnvelope,
  encodeControlFrameEnvelope,
} from "./wire.js";

const fixture = JSON.parse(
  readFileSync(new URL("../web-control/typed-frame-fixture.json", import.meta.url), "utf8"),
);
const wasmBytes = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));
const shell = await loadControlShell(wasmBytes);
shell.beginSession();
shell.authorityEvent("motion", "grant", { generation: 1n });
shell.authorityEvent("gimbal", "grant", { generation: 1n });
shell.beginControlRun();

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
  mode: "quad-pilot",
  connected: true,
  nowMs: 100_000,
};
const capability = fixture.capability;

// Keyboard chain: W (climb) through the real wasm to the typed envelope.
shell.keyEvent("w", true);
const keyPlan = shell.tickFromKeys(session);
const keyVelocity = buildVelocityIntent(keyPlan.motion, "quad-pilot", capability);
shell.clearKeys();
check("keyboard W commands full climb in the advertised envelope", keyVelocity.vz === -1.5, keyVelocity);

// Setpoint frames carry the intent ONLY: discrete actions ride the
// reliable session stream as ControlActionCommand, never datagrams.
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
});
const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
check("the typed envelope matches the shared fixture byte for byte", hex === fixture.envelopeHex, hex);

// The reliable action command for the same session: Arm bound to the full
// authority (session, vehicle, scope, generation, activation revision) with
// a nonzero correlation id.
const commandBytes = encodeControlActionCommandEnvelope({
  sessionId: 7,
  vehicleId: 1n,
  scope: "vehicle.motion",
  generation: 4n,
  activationRevision: shell.activationRevision(),
  action: CONTROL_ACTION.arm,
  actionId: 1,
});
const commandHex = Array.from(commandBytes, (b) => b.toString(16).padStart(2, "0")).join("");
check(
  "the action command matches the shared fixture byte for byte",
  commandHex === fixture.actionCommandHex,
  commandHex,
);

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
  intentCapabilityFor(
    [{ vehicleId: 1n, scope: "vehicle.motion", intents: [] }],
    1n,
    "vehicle.motion",
    1,
  ) === null,
);
check(
  "capability lookup is vehicle-scoped: another vehicle's scope never matches",
  intentCapabilityFor(
    [{ vehicleId: 2n, scope: "vehicle.motion", intents: [{ family: 1 }] }],
    1n,
    "vehicle.motion",
    1,
  ) === null,
);

// Gimbal rates scale by the advertised angular envelope.
const gimbalRate = buildGimbalRateIntent({ pitch: -1, yaw: 0.5 }, { maxAngular: 0.8 });
check(
  "gimbal rates scale by the advertised envelope",
  gimbalRate.pitchRate === -0.8 && gimbalRate.yawRate === 0.4,
  gimbalRate,
);

// Direct flight (CTRL-01 vehicle.motion.direct): tilt scales by the
// advertised bound, heading is the client-integrated setpoint, and the
// collective is the linear normalization with hover at 0.5.
const attitudeCapability = { maxAngular: 0.6, maxYawRate: 0.9 };
const level = buildAttitudeThrustIntent(
  { roll: 0, pitch: 0, throttle: 0, yaw: 0 },
  0,
  attitudeCapability,
);
check(
  "neutral sticks build the identity attitude at hover collective",
  level.qw === 1 && level.qx === 0 && level.qy === 0 && level.qz === 0 && level.thrust === 0.5,
  level,
);
const tilted = buildAttitudeThrustIntent(
  { roll: 1, pitch: 0, throttle: 1, yaw: 0 },
  0,
  attitudeCapability,
);
const rollBack = 2 * Math.atan2(tilted.qx, tilted.qw);
check(
  "full right stick commands exactly the advertised tilt bound",
  Math.abs(rollBack - 0.6) < 1e-6 && tilted.thrust === 1,
  { rollBack, thrust: tilted.thrust },
);
check(
  "no attitude advertisement yields no intent",
  buildAttitudeThrustIntent({ roll: 1, pitch: 0, throttle: 0, yaw: 0 }, 0, null) === null,
);
const slewed = integrateHeading(0, 1, attitudeCapability, 0.5);
check("the yaw stick slews the heading at the advertised rate", Math.abs(slewed - 0.45) < 1e-6, slewed);
check(
  "the heading setpoint wraps at pi",
  Math.abs(wrapHeading(Math.PI + 0.1) - (-Math.PI + 0.1)) < 1e-6,
  wrapHeading(Math.PI + 0.1),
);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("typed-command chain passed (browser half)");
