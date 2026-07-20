// Golden-vector parity for the control runtime: the SAME raw samples and the
// built-in profile must produce the SAME plan through the wasm as the native
// Rust `golden_vectors` test asserts (clients/web-control/src/golden.rs). This
// file drives the real wasm artifact; the two suites share the vectors below,
// so a native/wasm divergence reddens one of them.
//
// Build the wasm first: scripts/build-web-instruments.sh

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";

const wasmBytes = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));
const shell = await loadControlShell(wasmBytes);

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const session = (mode, granted = false, denied = false) => ({
  generation: 1,
  mode,
  connected: true,
  leaseGranted: granted,
  leaseDenied: denied,
  // The motion lease is granted in steady flight; the runtime gates motion
  // output when it is not (a profile handover fences it — see vector 8).
  motionGranted: true,
  motionDenied: false,
  nowMs: 100_000,
});
const pad = (axes, pressed) => ({
  axes,
  buttons: Array.from({ length: 16 }, (_, i) => ({
    pressed: pressed.includes(i),
    value: pressed.includes(i) ? 1 : 0,
  })),
});

// The runtime bootstrapped through the default profile.
check("the built-in default profile activated (revision 1)", shell.activationRevision() === 1);

// Vector 1: LT (button 6) held + right stick full → gimbal rates (pitch
// inverted), and flight sees the captured right stick as neutral.
{
  const plan = shell.tickFromPad(pad([0, 0, 1, 1], [6]), session("quad-pilot", true));
  check("v1 gimbal pitch is camera-down (inverted)", plan.gimbal?.pitch === -1);
  check("v1 gimbal yaw is camera-right", plan.gimbal?.yaw === 1);
  check("v1 flight roll is masked to neutral", plan.motion?.roll === 0);
  check("v1 flight pitch is masked to neutral", plan.motion?.pitch === 0);
  check("v1 no lease action while granted", plan.lease === null);
}

// Vector 2: a fresh R3 (button 11) press recenters exactly once.
{
  const first = shell.tickFromPad(pad([0, 0, 0, 0], [11]), session("quad-pilot", true));
  check("v2 a fresh R3 recenters", first.gimbal?.recenter === true);
  const second = shell.tickFromPad(pad([0, 0, 0, 0], [11]), session("quad-pilot", true));
  check("v2 holding R3 does not re-recenter", second.gimbal?.recenter === false);
}

// Vector 3: a flight mode with no lease requests it.
{
  const plan = shell.tickFromPad(pad([0, 0, 0, 0], []), session("quad-pilot", false));
  check("v3 a flight mode requests the gimbal lease", plan.lease === "request");
  check("v3 no gimbal frame without a lease", plan.gimbal === null);
}

// Vector 4: rover releases a held lease.
{
  const plan = shell.tickFromPad(pad([0, 0, 0, 0], []), session("rover", true));
  check("v4 rover releases the gimbal lease", plan.lease === "release");
}

// Vector 5: keyboard is a first-class source through the SAME runtime, matched
// on the shell's stored key form (event.key, letters lower-cased): W climbs.
{
  const plan = shell.tickFromKeys(new Set(["w"]), session("quad-pilot", true));
  check("v5 keyboard W commands climb", plan.motion?.throttle === 1);
}

// Vector 6: LT held with a centered stick still reports capture (HUD #167).
{
  const pad6 = pad([0, 0, 0, 0], [6]); // LT only, stick centered
  const plan = shell.tickFromPad(pad6, session("quad-pilot", true));
  check("v6 capture is reported at centered stick", plan.captureActive === true);
}

// Vector 7: a control held across a reconnect fires no edge — exercised through
// the REAL wasm runtime (the production path), not a JS helper. arm=button 9,
// R3=button 11; a fresh session generation seeds the baselines.
{
  const held = pad([0, 0, 0, 0], [9, 11]);
  const gen = (g) => ({ ...session("quad-pilot", true), generation: g });
  shell.tickFromPad(held, gen(100)); // establish generation 100 with the controls held
  const reconnect = shell.tickFromPad(held, gen(101)); // reconnect: new generation, still held
  check("v7 held arm across reconnect fires no arm", reconnect.arm === false);
  check("v7 held R3 across reconnect fires no recenter", reconnect.gimbal?.recenter === false);
  shell.tickFromPad(pad([0, 0, 0, 0], []), gen(101)); // release
  const press = shell.tickFromPad(held, gen(101)); // press again
  check("v7 a fresh arm after release fires once", press.arm === true);
  check("v7 a fresh R3 after release recenters once", press.gimbal?.recenter === true);
}

// Vector 8: motion output is GATED whenever the motion lease is not granted, so
// a remapped scheme can never publish a flight command on the released
// generation during a profile handover (the round-5 safety fix). This drives
// the session-bit-3 ABI through the real wasm: a fully deflected stick with no
// motion grant yields NO motion frame, and motion resumes once regranted.
{
  const stick = pad([1, 0, 0, 0], []); // left stick hard over
  const ungranted = { ...session("quad-pilot", true), motionGranted: false };
  check("v8 motion is gated without a motion-lease grant", shell.tickFromPad(stick, ungranted).motion === null);
  const granted = { ...session("quad-pilot", true), motionGranted: true };
  check("v8 motion resumes once the motion lease is granted", shell.tickFromPad(stick, granted).motion !== null);
}

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all control-runtime golden vectors passed");
