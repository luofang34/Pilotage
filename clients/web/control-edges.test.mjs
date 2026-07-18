// Checks for arm/disarm edge detection: rising edges only, and — the safety
// property — a button held across a (re)connect or through a focus loss does
// not fire a fresh arm/disarm once the previous set is primed/cleared.
//
// Run: node clients/web/control-edges.test.mjs

import { pressedArmInputs, risingArmEdges } from "./control-edges.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const arr = (set) => [...set].sort();

// ---- pressedArmInputs maps raw booleans to a stable set --------------------
{
  check("nothing held is an empty set", pressedArmInputs({}).size === 0);
  const held = pressedArmInputs({ padArm: true, keyDisarm: true });
  check(
    "held pad-arm + key-disarm map through",
    arr(held).join(",") === "key-disarm,pad-arm",
  );
}

// ---- rising edges only ------------------------------------------------------
{
  const now = new Set(["pad-arm"]);
  check("a newly pressed input is a rising edge", risingArmEdges(now, new Set()).join() === "pad-arm");
  check(
    "an input held since last tick is NOT an edge",
    risingArmEdges(now, new Set(["pad-arm"])).length === 0,
  );
  const two = new Set(["pad-arm", "key-disarm"]);
  check(
    "only the newly-pressed input edges",
    risingArmEdges(two, new Set(["pad-arm"])).join() === "key-disarm",
  );
}

// ---- the hold-across-reconnect safety property ------------------------------
{
  // The arm button is held down the whole time. At (re)connect the previous set
  // is PRIMED to the currently-held inputs, so the first tick sees no edge.
  const heldNow = pressedArmInputs({ padArm: true });
  const primed = pressedArmInputs({ padArm: true }); // priming reads the same held state
  check(
    "a button held across reconnect does not edge when primed",
    risingArmEdges(heldNow, primed).length === 0,
  );

  // The wrong (old) behavior: priming to an empty set turns the held button
  // into a spurious arm on the first tick. This is the bug the priming fixes.
  check(
    "priming to empty would (wrongly) edge — the bug this prevents",
    risingArmEdges(heldNow, new Set()).join() === "pad-arm",
  );

  // After a genuine release and re-press, it edges again.
  const afterRelease = new Set();
  const rePressed = pressedArmInputs({ padArm: true });
  check(
    "releasing then pressing again edges",
    risingArmEdges(rePressed, afterRelease).join() === "pad-arm",
  );
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall control-edge checks passed");
