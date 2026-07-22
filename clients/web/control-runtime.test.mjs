// Executes the SHARED golden-vector file (clients/web-control/golden-vectors.json)
// through the REAL compiled wasm artifact. The native suite
// (clients/web-control/src/golden.rs) executes the SAME file, so the vectors
// cannot drift: a native/wasm divergence reddens exactly one of the two.
//
// Build the wasm first: scripts/build-web-instruments.sh

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";

const wasmBytes = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));
const vectors = JSON.parse(
  readFileSync(new URL("../web-control/golden-vectors.json", import.meta.url), "utf8"),
);

// Both harnesses build pad samples with this many buttons, so pressed-set
// semantics match bit for bit.
const PAD_BUTTONS = 16;

let failures = 0;
function check(name, ok, got) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}${got === undefined ? "" : ` (got ${JSON.stringify(got)})`}`);
  }
}

function toSession(session) {
  return {
    generation: session.generation ?? 1,
    mode: session.mode,
    connected: true,
    leaseGranted: session.gimbalGranted ?? false,
    leaseDenied: session.gimbalDenied ?? false,
    motionGranted: session.motionGranted ?? true,
    motionDenied: session.motionDenied ?? false,
    motionRecovered: session.motionRecovered ?? true,
    nowMs: 100_000,
  };
}

function toPad(pad) {
  return {
    axes: pad.axes,
    buttons: Array.from({ length: PAD_BUTTONS }, (_, i) => ({
      pressed: pad.pressed.includes(i),
      value: pad.pressed.includes(i) ? 1 : 0,
    })),
  };
}

function axisValue(frame, name) {
  return frame ? frame[name] : Number.NaN;
}

function checkPlan(expect, plan, ctx) {
  if (expect.motionGated === true) check(`${ctx}: motion gated`, plan.motion === null, plan.motion);
  if (expect.motionLive === true) check(`${ctx}: motion live`, plan.motion !== null, plan.motion);
  for (const [key, name] of [
    ["motionRoll", "roll"],
    ["motionPitch", "pitch"],
    ["motionThrottle", "throttle"],
    ["motionYaw", "yaw"],
  ]) {
    if (expect[key] !== undefined) {
      const got = axisValue(plan.motion, name);
      check(`${ctx}: motion ${name}`, got === expect[key], got);
    }
  }
  if (expect.gimbalNull === true) check(`${ctx}: gimbal absent`, plan.gimbal === null, plan.gimbal);
  if (expect.gimbalPitch !== undefined) {
    const got = axisValue(plan.gimbal, "pitch");
    check(`${ctx}: gimbal pitch`, got === expect.gimbalPitch, got);
  }
  if (expect.gimbalYaw !== undefined) {
    const got = axisValue(plan.gimbal, "yaw");
    check(`${ctx}: gimbal yaw`, got === expect.gimbalYaw, got);
  }
  if (expect.recenter !== undefined) {
    const fired = plan.gimbal?.recenter === true;
    check(`${ctx}: recenter edge`, fired === expect.recenter, fired);
  }
  if (expect.lease !== undefined) {
    const got = plan.lease ?? "none";
    check(`${ctx}: lease action`, got === expect.lease, got);
  }
  if (expect.capture !== undefined) {
    check(`${ctx}: capture`, plan.captureActive === expect.capture, plan.captureActive);
  }
  if (expect.arm !== undefined) check(`${ctx}: arm edge`, plan.arm === expect.arm, plan.arm);
  if (expect.disarm !== undefined) {
    check(`${ctx}: disarm edge`, plan.disarm === expect.disarm, plan.disarm);
  }
  if (expect.armSuppressed !== undefined) {
    check(`${ctx}: arm suppressed`, plan.armSuppressed === expect.armSuppressed, plan.armSuppressed);
  }
  if (expect.disarmSuppressed !== undefined) {
    check(
      `${ctx}: disarm suppressed`,
      plan.disarmSuppressed === expect.disarmSuppressed,
      plan.disarmSuppressed,
    );
  }
  if (expect.motionLease !== undefined) {
    const got = plan.motionLease ?? "none";
    check(`${ctx}: motion lease action`, got === expect.motionLease, got);
  }
}

function runStep(shell, step, ctx) {
  const expect = step.expect ?? {};
  if (step.selectDevice !== undefined) {
    const got = shell.selectDevice(step.selectDevice) ?? "refused";
    if (expect.select !== undefined) {
      check(`${ctx}: selection outcome`, got === expect.select, got);
    }
  }
  if (step.addDeviceProfile !== undefined) {
    const bytes = new TextEncoder().encode(JSON.stringify(step.addDeviceProfile.profile));
    const added = shell.addDeviceProfile(step.addDeviceProfile.layer, bytes);
    if (expect.added !== undefined) {
      check(`${ctx}: profile added`, added === expect.added, added);
    }
  }
  if (step.expectBound) {
    for (const [key, want] of Object.entries(step.expectBound)) {
      const got = shell.boundKey(key);
      check(`${ctx}: key ${key} bound`, got === want, got);
    }
  }
  for (const [key, pressed] of step.keyEvents ?? []) {
    shell.keyEvent(key, pressed);
  }
  if (step.clearKeys) shell.clearKeys();
  if (step.session) {
    const session = toSession(step.session);
    const plan = step.pad
      ? shell.tickFromPad(toPad(step.pad), session)
      : shell.tickFromKeys(session);
    if (step.expect) checkPlan(step.expect, plan, ctx);
  }
  // Label and revision reflect the INSTALLED state, so they are checked
  // after any tick this step ran (a pending swap installs mid-tick).
  if (expect.label !== undefined) {
    const label = shell.deviceLabel();
    check(`${ctx}: device label`, label === expect.label, label);
  }
  if (expect.activationRevision !== undefined) {
    const revision = shell.activationRevision();
    check(`${ctx}: activation revision`, revision === expect.activationRevision, revision);
  }
}

for (const group of vectors.groups) {
  // Each group runs on a fresh runtime bootstrapped through the default
  // profile — the same reset rule the native harness applies.
  const shell = await loadControlShell(wasmBytes);
  check(`${group.name}: default profile activated`, shell.activationRevision() === 1);
  group.steps.forEach((step, index) => {
    runStep(shell, step, `${group.name} / step ${index + 1}`);
  });
}

// Same-session resume rides the authority-recovery contract: the regrant's
// fresh generation gates a held deflection, retransmits neutral once
// centered, and goes live only after the host confirms link-loss clearance.
// Every negative check has a positive control on the SAME key, so a dud key
// name cannot make the suite pass vacuously.
{
  const shell = await loadControlShell(wasmBytes);
  const deflects = (motion) =>
    motion !== null && Object.values(motion).some((value) => value !== 0);
  const isNeutral = (motion) =>
    motion !== null && Object.values(motion).every((value) => value === 0);
  const live = toSession({ mode: "quad-pilot", generation: 1, motionRecovered: true });
  shell.tickFromKeys(live);
  shell.keyEvent("ArrowUp", true);
  check(
    "same-session resume: the bound key deflects while live (positive control)",
    deflects(shell.tickFromKeys(live).motion),
  );

  const suspended = toSession({
    mode: "quad-pilot",
    generation: 1,
    motionGranted: false,
    motionRecovered: false,
  });
  check("same-session resume: suspension gates motion", shell.tickFromKeys(suspended).motion === null);
  // The arm key goes down while control is suspended (no ticks run).
  shell.keyEvent("Enter", true);

  const regranted = toSession({
    mode: "quad-pilot",
    generation: 2,
    motionRecovered: false,
  });
  const primed = shell.tickFromKeys(regranted);
  check(
    "same-session resume: a held deflection cannot publish on regrant",
    primed.motion === null,
    primed.motion,
  );
  check(
    "same-session resume: an arm pressed while suspended fires no edge",
    primed.arm === false,
  );
  shell.clearKeys();
  check(
    "same-session resume: centered controls retransmit the neutral activation",
    isNeutral(shell.tickFromKeys(regranted).motion),
  );
  shell.keyEvent("ArrowUp", true);
  check(
    "same-session resume: re-deflecting mid-recovery gates again",
    shell.tickFromKeys(regranted).motion === null,
  );

  const confirmed = toSession({ mode: "quad-pilot", generation: 2, motionRecovered: true });
  check(
    "same-session resume: confirmation completes with one final neutral",
    isNeutral(shell.tickFromKeys(confirmed).motion),
  );
  check(
    "same-session resume: live resumes after confirmation (positive control)",
    deflects(shell.tickFromKeys(confirmed).motion),
  );
  shell.keyEvent("Enter", false);
  shell.keyEvent("Enter", true);
  check(
    "same-session resume: a fresh arm press after recovery arms (positive control)",
    shell.tickFromKeys(confirmed).arm === true,
  );
}

// Operator-facing arm/disarm hints come from profile data, renamed by the
// active source: the keyboard names its bound keys, a pad its printed
// buttons — never a hardcoded string in the shell.
{
  const shell = await loadControlShell(wasmBytes);
  check(
    "control hints: the keyboard names its bound keys",
    shell.armHint() === "Enter" && shell.disarmHint() === "Backspace",
    `${shell.armHint()}/${shell.disarmHint()}`,
  );
  shell.selectDevice("DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)");
  shell.tickFromKeys(toSession({ mode: "quad-pilot" }));
  check(
    "control hints: the selected pad names its printed buttons",
    shell.armHint() === "Options" && shell.disarmHint() === "Create",
    `${shell.armHint()}/${shell.disarmHint()}`,
  );
}

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all shared control-runtime golden vectors passed (wasm)");
