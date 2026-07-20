// Checks for the gimbal quasimode (GIM-03, #167). The safety properties:
// while LT is held, flight must see the right stick AND LT as neutral (no
// scheme may consume a captured input — LT-descend included, so RT still
// climbs); entering/leaving the quasimode neutralizes the scope it leaves
// (exit yields exactly one trailing neutral gimbal frame); R3 recenter is a
// one-shot edge, never a level.
//
// Run: node clients/web/gimbal-input.test.mjs

import {
  PAD_GIMBAL_MODIFIER,
  gimbalAxesFromGamepad,
  gimbalFramePlan,
  gimbalLeasePlan,
  gimbalMaskedView,
  gimbalModifierHeld,
  gimbalResetEdge,
  stickShaper,
} from "./gimbal-input.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const STANDARD = { standard: true, deadzone: 0.06 };

function pad({ axes = [0, 0, 0, 0], buttons = [] } = {}) {
  const full = Array.from({ length: 12 }, (_, i) => buttons[i] ?? { pressed: false, value: 0 });
  return { id: "test pad", connected: true, axes, buttons: full };
}

// ---- modifier detection -----------------------------------------------------
{
  const held = pad({ buttons: { [PAD_GIMBAL_MODIFIER]: { pressed: false, value: 0.9 } } });
  check("LT analog past half travel engages", gimbalModifierHeld(held, STANDARD));
  const digital = pad({ buttons: { [PAD_GIMBAL_MODIFIER]: { pressed: true, value: 0 } } });
  check("LT digital press engages", gimbalModifierHeld(digital, STANDARD));
  const light = pad({ buttons: { [PAD_GIMBAL_MODIFIER]: { pressed: false, value: 0.3 } } });
  check("a light LT touch does not engage", !gimbalModifierHeld(light, STANDARD));
  check(
    "non-standard layouts (EdgeTX) never engage",
    !gimbalModifierHeld(held, { standard: false }),
  );
  check("no pad never engages", !gimbalModifierHeld(null, STANDARD));
}

// ---- capture masking: flight sees neutral where the quasimode captured -----
{
  const busy = pad({
    axes: [0.4, -0.2, 0.8, -0.9],
    buttons: {
      6: { pressed: true, value: 1.0 }, // LT: captured (descend must NOT fire)
      7: { pressed: true, value: 0.7 }, // RT: must pass through (climb keeps working)
      9: { pressed: true, value: 1.0 }, // options/arm: must pass through
    },
  });
  const masked = gimbalMaskedView(busy);
  check("right stick X reads neutral to flight", masked.axes[2] === 0);
  check("right stick Y reads neutral to flight", masked.axes[3] === 0);
  check("left stick passes through", masked.axes[0] === 0.4 && masked.axes[1] === -0.2);
  check("LT reads neutral to flight (descend inhibited)", masked.buttons[6].value === 0 && !masked.buttons[6].pressed);
  check("RT passes through (climb keeps working)", masked.buttons[7].value === 0.7);
  check("arm button passes through", masked.buttons[9].pressed === true);
}

// ---- gimbal axes: signs and shaping ----------------------------------------
{
  // Direct sense (owner decision after flying both): stick up = camera up.
  const up = pad({ axes: [0, 0, 0, -1] }); // stick up = browser negative
  check("stick up = camera up at full rate", gimbalAxesFromGamepad(up, STANDARD).pitch === 1);
  const down = pad({ axes: [0, 0, 0, 1] });
  check("stick down = camera down at full rate", gimbalAxesFromGamepad(down, STANDARD).pitch === -1);
  const right = pad({ axes: [0, 0, 1, 0] });
  check("stick right = camera right at full rate", gimbalAxesFromGamepad(right, STANDARD).yaw === 1);
  const inside = pad({ axes: [0, 0, 0.04, -0.04] });
  const g = gimbalAxesFromGamepad(inside, STANDARD);
  check("deadzone zeroes small deflections", g.pitch === 0 && g.yaw === 0);
  const shaped = stickShaper(STANDARD)(0.5);
  check("expo softens mid-stick", shaped > 0 && shaped < 0.5);
}

// ---- frame plan: entry/exit neutralization and the one-shot recenter -------
{
  check(
    "idle: no frame at all",
    gimbalFramePlan({ held: false, resetEdge: false, streaming: false, rates: { pitch: 1, yaw: 1 } }) === null,
  );
  const active = gimbalFramePlan({ held: true, resetEdge: false, streaming: false, rates: { pitch: 0.5, yaw: -0.5 } });
  check("held: streams the stick rates", active.rates.pitch === 0.5 && active.streaming === true);
  const exit = gimbalFramePlan({ held: false, resetEdge: false, streaming: true, rates: { pitch: 0.5, yaw: -0.5 } });
  check(
    "exit: exactly one trailing NEUTRAL frame, stick value discarded",
    exit.rates.pitch === 0 && exit.rates.yaw === 0 && exit.streaming === false,
  );
  const afterExit = gimbalFramePlan({ held: false, resetEdge: false, streaming: exit.streaming, rates: { pitch: 0.5, yaw: 0 } });
  check("after the trailing frame the stream stays closed", afterExit === null);
  const recenter = gimbalFramePlan({ held: false, resetEdge: true, streaming: false, rates: { pitch: 0.9, yaw: 0.9 } });
  check(
    "R3 recenter fires without opening a rate stream",
    recenter.recenter === true && recenter.rates.pitch === 0 && recenter.streaming === false,
  );
}

// ---- R3 recenter edge: baseline always advances, no false edge --------------
{
  // A held R3 while inactive advances the baseline but fires no edge...
  const held1 = gimbalResetEdge(true, false, false);
  check("R3 held while inactive fires no edge", held1.edge === false);
  check("R3 baseline advances even while inactive", held1.prevHeld === true);
  // ...so when the gimbal path re-activates with R3 still held, no false edge.
  const held2 = gimbalResetEdge(true, held1.prevHeld, true);
  check("R3 held across (re)activation is NOT a fresh edge", held2.edge === false);
  // A genuine press while active fires exactly once.
  const press = gimbalResetEdge(true, false, true);
  check("a fresh R3 press while active fires the edge", press.edge === true);
  const stillHeld = gimbalResetEdge(true, press.prevHeld, true);
  check("holding R3 does not re-fire the edge", stillHeld.edge === false);
}

// ---- gimbal lease state machine ---------------------------------------------
{
  const base = { granted: false, denied: false, requestedAtMs: 0, nowMs: 5000 };
  check(
    "flight mode with no lease requests it",
    gimbalLeasePlan({ ...base, mode: "quad-pilot" }) === "request",
  );
  check(
    "a fresh request is debounced",
    gimbalLeasePlan({ ...base, mode: "quad-pilot", requestedAtMs: 4000 }) === "none",
  );
  check(
    "a granted lease is not re-requested",
    gimbalLeasePlan({ ...base, mode: "quad-pilot", granted: true }) === "none",
  );
  check(
    "a denied scope is never re-requested",
    gimbalLeasePlan({ ...base, mode: "quad-pilot", denied: true }) === "none",
  );
  check(
    "rover mode releases a held lease",
    gimbalLeasePlan({ ...base, mode: "rover", granted: true }) === "release",
  );
  check(
    "rover mode with no lease does nothing",
    gimbalLeasePlan({ ...base, mode: "rover" }) === "none",
  );
}

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all gimbal-input checks passed");
