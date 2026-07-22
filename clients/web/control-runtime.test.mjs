// Executes the SHARED golden-vector file (clients/web-control/golden-vectors.json)
// through the REAL compiled wasm artifact. The native suite
// (clients/web-control/src/golden.rs) executes the SAME file, so the vectors
// cannot drift: a native/wasm divergence reddens exactly one of the two.
//
// Build the wasm first: scripts/build-web-instruments.sh

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";
import { applyAuthorityTransition } from "./authority-transition.js";

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
    mode: session.mode,
    connected: true,
    inputLost: false,
    nowMs: 100_000,
  };
}

// A latched input-loss tick consumes controls but gives the shell no
// publishable frame, edge, or lease action, even with live authority.
{
  const shell = await loadControlShell(wasmBytes);
  shell.beginSession();
  shell.authorityEvent("motion", "grant", { generation: 1n });
  shell.authorityEvent("gimbal", "grant", { generation: 1n });
  shell.beginControlRun();
  const live = toSession({ mode: "quad-pilot" });
  shell.tickFromKeys(live);
  const lost = { ...live, inputLost: true };
  const empty = shell.tickFromKeys(lost);
  check(
    "input loss: a granted live session emits an empty publishable plan",
    empty.motion === null && empty.gimbal === null && empty.lease === null &&
      empty.motionLease === null && !empty.arm && !empty.disarm,
    empty,
  );
  shell.keyEvent("Enter", true);
  const pressed = shell.tickFromKeys(lost);
  check("input loss: a consumed arm press remains loud", pressed.armSuppressed, pressed);
  const resumed = shell.tickFromKeys(live);
  check("input loss: the consumed press cannot re-fire on resume", !resumed.arm, resumed);
  shell.keyEvent("Enter", false);
}

function syncScope(shell, scope, granted, denied, generation) {
  const current = shell.authority(scope);
  if (denied && !current.denied) {
    shell.authorityEvent(scope, "denial");
  } else if (granted && !current.granted && !current.denied) {
    const result = shell.authorityEvent(scope, "grant", { generation });
    if (result === "stale") {
      shell.authorityEvent(scope, "grant", { generation: current.generation + 1n });
    }
  } else if (!granted && current.granted) {
    shell.authorityEvent(scope, "release", { generation: current.generation });
  }
}

function syncAuthority(shell, session, tracker) {
  const generation = BigInt(session.generation ?? 1);
  const freshSession = tracker.generation !== generation;
  if (freshSession) {
    shell.beginSession();
    tracker.generation = generation;
  }
  syncScope(
    shell,
    "motion",
    session.motionGranted ?? true,
    session.motionDenied ?? false,
    generation,
  );
  syncScope(
    shell,
    "gimbal",
    session.gimbalGranted ?? false,
    session.gimbalDenied ?? false,
    generation,
  );
  const motion = shell.authority("motion");
  if ((session.motionRecovered ?? true) && motion.granted && !motion.recovered) {
    shell.authorityEvent("motion", "recovery", { generation: motion.generation });
  }
  if (freshSession) shell.beginControlRun();
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

function runStep(shell, step, ctx, authorityTracker) {
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
    syncAuthority(shell, step.session, authorityTracker);
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
  const authorityTracker = { generation: null };
  group.steps.forEach((step, index) => {
    runStep(shell, step, `${group.name} / step ${index + 1}`, authorityTracker);
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
  const live = toSession({ mode: "quad-pilot" });
  shell.beginSession();
  shell.authorityEvent("motion", "grant", { generation: 1n });
  shell.beginControlRun();
  shell.tickFromKeys(live);
  shell.keyEvent("ArrowUp", true);
  check(
    "same-session resume: the bound key deflects while live (positive control)",
    deflects(shell.tickFromKeys(live).motion),
  );

  shell.authorityEvent("motion", "release", { generation: 1n });
  const suspended = toSession({ mode: "quad-pilot" });
  check("same-session resume: suspension gates motion", shell.tickFromKeys(suspended).motion === null);
  // The arm key goes down while control is suspended (no ticks run).
  shell.keyEvent("Enter", true);

  shell.authorityEvent("motion", "grant", { generation: 2n });
  shell.beginControlRun();
  const regranted = toSession({ mode: "quad-pilot" });
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

  shell.authorityEvent("motion", "recovery", { generation: 2n });
  const confirmed = toSession({ mode: "quad-pilot" });
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

// The wasm seam mirrors the native table guardrails: every scope fences,
// denial is terminal, modular wrap is fresh, and enactment truth latches.
{
  const shell = await loadControlShell(wasmBytes);
  for (const scope of ["motion", "gimbal", "lifecycle"]) {
    shell.beginSession();
    shell.authorityEvent(scope, "grant", { generation: 7n });
    shell.authorityEvent(scope, "release", { generation: 9n });
    check(
      `authority table: ${scope} rejects its fenced generation`,
      shell.authorityEvent(scope, "grant", { generation: 9n }) === "stale" &&
        shell.authority(scope).granted === false,
    );
    shell.authorityEvent(scope, "grant", { generation: 10n });
    check(
      `authority table: ${scope} ignores a delayed older fence`,
      shell.authorityEvent(scope, "release", { generation: 9n }) === "ignored" &&
        shell.authority(scope).generation === 10n,
    );
  }

  shell.beginSession();
  shell.authorityEvent("motion", "grant", { generation: (1n << 64n) - 1n });
  shell.authorityEvent("motion", "release", { generation: (1n << 64n) - 1n });
  check(
    "authority table: generation wrap is accepted",
    shell.authorityEvent("motion", "grant", { generation: 0n }) === "applied",
  );
  shell.authorityEvent("motion", "denial");
  check(
    "authority table: denial is terminal for the session",
    shell.authorityEvent("motion", "grant", { generation: 1n }) === "ignored" &&
      shell.authority("motion").denied,
  );

  shell.beginSession();
  shell.authorityEvent("motion", "uplinkIdle");
  check("authority table: idle uplink needs arm", shell.authority("motion").needsArm);
  shell.authorityEvent("motion", "actionResult", { detail: 1, accepted: true });
  check(
    "authority table: accepted arm clears needs-arm",
    shell.authority("motion").needsArm === false,
  );
  shell.authorityEvent("motion", "actionResult", { detail: 2, accepted: true });
  check("authority table: accepted disarm restores needs-arm", shell.authority("motion").needsArm);
}

// The unicast response and broadcast stream report the same host transition.
// Only the table-confirmed first arrival is operator-visible.
{
  const shell = await loadControlShell(wasmBytes);
  const lines = [];
  const apply = (kind, generation) =>
    applyAuthorityTransition(shell, (line) => lines.push(line), "motion", kind, { generation });
  shell.beginSession();
  apply("grant", 11n);
  apply("grant", 11n);
  check(
    "authority logging: a unicast and broadcast grant produce one line",
    lines.filter((line) => line === "authority[motion]: granted gen=11").length === 1,
    lines,
  );
  apply("release", 12n);
  apply("revocation", 12n);
  check(
    "authority logging: a release and revocation replay produce one line",
    lines.filter((line) => line.includes("fence=12")).length === 1,
    lines,
  );
  apply("grant", 12n);
  check(
    "authority logging: a stale grant is loud",
    lines.at(-1) === "authority[motion]: STALE grant gen=12",
    lines.at(-1),
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
